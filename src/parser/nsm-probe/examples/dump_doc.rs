use qos_nsm::nitro;
use x509_cert::der::Decode as _;

fn main() {
    let path = std::env::args().nth(1).unwrap_or_default();
    let bytes = std::fs::read(&path).unwrap_or_default();
    let Ok(doc) = nitro::unsafe_attestation_doc_from_der(&bytes) else {
        eprintln!("decode failed");
        return;
    };
    println!(
        "timestamp_ms={} module_id={} digest={:?}",
        doc.timestamp, doc.module_id, doc.digest
    );
    println!(
        "user_data={:?} public_key_len={:?} nonce={:?}",
        doc.user_data.as_ref().map(|u| qos_hex::encode(u)),
        doc.public_key.as_ref().map(|k| k.len()),
        doc.nonce
    );
    let certs = std::iter::once(("leaf".to_string(), doc.certificate.as_slice())).chain(
        doc.cabundle
            .iter()
            .enumerate()
            .map(|(i, c)| (format!("cabundle[{i}]"), c.as_slice())),
    );
    for (name, der) in certs {
        if let Ok(c) = x509_cert::Certificate::from_der(der) {
            let v = &c.tbs_certificate.validity;
            println!(
                "{name}: not_before={} not_after={} ({}s) subject={}",
                v.not_before,
                v.not_after,
                v.not_after.to_unix_duration().as_secs(),
                c.tbs_certificate.subject
            );
        }
    }
}
