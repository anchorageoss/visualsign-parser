// Signed payment marker for parser_http_server's --require-payment-marker-pubkey.
//
// Byte-for-byte mirror of host_primitives::payment_marker: borsh of
// VerifiedPaymentMarker, SHA-256, then a P-256 ECDSA signature over that
// 32-byte digest. qos_p256 hashes the message again internally, and WebCrypto's
// ECDSA/SHA-256 does the same, so signing the digest here matches. The
// signature is raw r||s (IEEE P1363), 64 bytes, which is what WebCrypto emits.

const VPM_VERSION = 1;
const enc = new TextEncoder();

// parser.proto Chain enum. Only chains the PoC has been exercised on.
export const CHAIN_NUMBERS = { CHAIN_ETHEREUM: 2, CHAIN_SOLANA: 3 };

function u32(n) {
  const b = new Uint8Array(4);
  new DataView(b.buffer).setUint32(0, n, true);
  return b;
}

function u64(n) {
  const b = new Uint8Array(8);
  new DataView(b.buffer).setBigUint64(0, BigInt(n), true);
  return b;
}

function str(s) {
  const bytes = enc.encode(s);
  return concat(u32(bytes.length), bytes);
}

function concat(...parts) {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let off = 0;
  for (const p of parts) {
    out.set(p, off);
    off += p.length;
  }
  return out;
}

export async function sha256(bytes) {
  return new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
}

export function hex(bytes) {
  return [...bytes].map((b) => b.toString(16).padStart(2, "0")).join("");
}

export function b64(bytes) {
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin);
}

// host_primitives::payment_marker::request_hash. chain_metadata is supported
// only when absent: its preimage is the borsh of the Rust ChainMetadata type,
// which this Worker does not reproduce.
export async function requestHash(chain, unsignedPayload, includeIntermediateOutput) {
  const payload = enc.encode(unsignedPayload);
  return sha256(
    concat(
      u32(chain),
      u32(payload.length),
      payload,
      u32(0),
      Uint8Array.of(includeIntermediateOutput ? 1 : 0),
    ),
  );
}

// SEC1 uncompressed public key (04 || x || y) hex, from the private JWK.
export function pubkeyHexFromJwk(jwk) {
  const fromB64url = (s) => Uint8Array.from(atob(s.replace(/-/g, "+").replace(/_/g, "/")), (c) => c.charCodeAt(0));
  return hex(concat(Uint8Array.of(4), fromB64url(jwk.x), fromB64url(jwk.y)));
}

// Borsh of VerifiedPaymentMarker with PaymentDetails::X402Direct (variant 0).
export function encodeVpm(v) {
  return concat(
    u32(VPM_VERSION),
    v.requestHash,
    Uint8Array.of(0),
    str(v.txid),
    str(v.payer),
    str(v.payTo),
    str(v.amount),
    str(v.mint),
    v.xPaymentHash,
    str(v.network),
    u64(v.settledAtMs),
    str(v.gatewayPubkeyHex),
  );
}

// base64(borsh(SignedVerifiedPaymentMarker)): the vpm, then the signature as a
// length-prefixed Vec<u8>.
export async function signMarker(jwkJson, v) {
  const jwk = JSON.parse(jwkJson);
  const key = await crypto.subtle.importKey(
    "jwk",
    jwk,
    { name: "ECDSA", namedCurve: "P-256" },
    false,
    ["sign"],
  );
  const vpm = encodeVpm({ ...v, gatewayPubkeyHex: pubkeyHexFromJwk(jwk) });
  const digest = await sha256(vpm);
  const signature = new Uint8Array(
    await crypto.subtle.sign({ name: "ECDSA", hash: "SHA-256" }, key, digest),
  );
  return b64(concat(vpm, u32(signature.length), signature));
}
