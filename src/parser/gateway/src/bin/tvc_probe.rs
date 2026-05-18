//! One-off probe: gRPC + X-Stamp to a deployed Turnkey TVC app.
//!
//! Uses the same Turnkey API key file the Go `visualsign-turnkey-client`
//! uses (`~/.config/turnkey/keys/<name>.{private,public}`). For each
//! outbound gRPC request, encodes the message to protobuf bytes,
//! SHA-256-hashes them, ECDSA-P256-signs (DER), wraps in the `X-Stamp`
//! JSON, base64URL-encodes, and attaches as a gRPC metadata header.
//!
//! Usage:
//!   cargo run -p parser_gateway --bin tvc_probe -- <APP_URL> <ORG_ID> [KEY_NAME]
//!   e.g. tvc_probe https://app-<your-app-uuid>.turnkey.cloud <your-org-uuid> default

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use base64::Engine;
use generated::parser::Chain;
use generated::parser::ParseRequest;
use generated::parser::parser_service_client::ParserServiceClient;
use generated::tonic;
use p256::ecdsa::{Signature, SigningKey, signature::Signer};
use prost::Message;
use serde_json::json;
use std::time::Duration;
use tonic::Request;
use tonic::metadata::MetadataValue;
use tonic::transport::{Channel, ClientTlsConfig, Endpoint};

#[derive(Clone)]
struct Stamper {
    priv_hex: String,
    pub_hex: String,
}

impl Stamper {
    fn load(key_name: &str) -> Self {
        let home = std::env::var("HOME").expect("HOME unset");
        let priv_raw =
            std::fs::read_to_string(format!("{home}/.config/turnkey/keys/{key_name}.private"))
                .expect("read private key");
        // Strip optional ":p256" suffix per the Go client convention.
        let priv_hex = priv_raw.trim().split(':').next().unwrap().to_string();
        let pub_hex =
            std::fs::read_to_string(format!("{home}/.config/turnkey/keys/{key_name}.public"))
                .expect("read public key")
                .trim()
                .to_string();
        Self { priv_hex, pub_hex }
    }

    fn stamp(&self, body: &[u8]) -> String {
        let priv_bytes = hex_decode(&self.priv_hex);
        let key = SigningKey::from_slice(&priv_bytes).expect("bad p256 priv");
        // p256's Signer impl applies SHA-256 internally, matching the Go ref.
        let sig: Signature = key.sign(body);
        let der = sig.to_der().to_bytes();
        let sig_hex = hex_encode(&der);

        let stamp = json!({
            "publicKey": self.pub_hex,
            "signature": sig_hex,
            "scheme": "SIGNATURE_SCHEME_TK_API_P256",
        });
        let stamp_bytes = serde_json::to_vec(&stamp).unwrap();
        // base64url, no padding (Go uses RawURLEncoding)
        let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        engine.encode(stamp_bytes)
    }
}

fn hex_decode(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn hex_encode(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: tvc_probe <APP_URL> <ORG_ID> [KEY_NAME]");
        std::process::exit(2);
    }
    let app_url = args[1].clone();
    let _org_id = args[2].clone();
    let key_name = args
        .get(3)
        .cloned()
        .unwrap_or_else(|| "default".to_string());

    let stamper = Stamper::load(&key_name);
    eprintln!("loaded API key {key_name} pub={}", &stamper.pub_hex[..16]);

    // tonic 0.9 with feature `tls-roots` auto-includes native roots when a
    // default ClientTlsConfig is attached. The endpoint scheme drives TLS.
    let tls = ClientTlsConfig::new();
    let endpoint = Endpoint::from_shared(app_url.clone())?
        .tls_config(tls)?
        .user_agent("turnkey-grpc-probe/0.1")?
        .origin(app_url.parse()?)
        .timeout(Duration::from_secs(30));
    eprintln!("dialing {app_url} over TLS...");
    let channel: Channel = endpoint.connect().await?;

    let mut client = ParserServiceClient::new(channel);

    let req = ParseRequest {
        unsigned_payload: "0xf86c808504a817c800825208943535353535353535353535353535353535353535880de0b6b3a76400008025a028ef61340bd939bc2195fe537567866003e1a15d3c71ff63e1590620aa636276a067cbe9d8997f761aecb703304b3800ccf555c9f3dc64214b297fb1966a3b6d83".to_string(),
        chain: Chain::Ethereum as i32,
        chain_metadata: None,
        payment_marker: Vec::new(),
    };

    // Encode protobuf payload to compute the signature over.
    let proto_bytes = req.encode_to_vec();
    let stamp = stamper.stamp(&proto_bytes);
    eprintln!(
        "stamp header {} chars (first 40: {})",
        stamp.len(),
        &stamp[..40]
    );

    let mut request = Request::new(req);
    request
        .metadata_mut()
        .insert("x-stamp", MetadataValue::try_from(stamp.as_str()).unwrap());
    // Try a couple of likely Turnkey edge auth headers in case the gRPC
    // path expects something different than the HTTP/JSON path.
    if let Ok(v) = MetadataValue::try_from(args[2].as_str()) {
        request
            .metadata_mut()
            .insert("x-turnkey-organization-id", v.clone());
        request.metadata_mut().insert("x-organization-id", v);
    }

    eprintln!("sending Parse RPC...");
    match client.parse(request).await {
        Ok(resp) => {
            let r = resp.into_inner();
            eprintln!("OK");
            if let Some(parsed_tx) = r.parsed_transaction
                && let Some(payload) = parsed_tx.payload
            {
                println!("signable_payload (first 200 chars):");
                println!(
                    "{}",
                    &payload.parsed_payload[..200.min(payload.parsed_payload.len())]
                );
            }
        }
        Err(status) => {
            eprintln!("ERROR: code={:?}", status.code());
            eprintln!("message: {}", status.message());
            eprintln!("metadata: {:?}", status.metadata());
            std::process::exit(1);
        }
    }

    Ok(())
}
