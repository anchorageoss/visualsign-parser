// Generate the P-256 key that signs payment markers. The private JWK goes to
// OUT (default marker-key.jwk, mode 600) and is never printed; the public key
// (SEC1 uncompressed hex) is what the backend pins with
// --require-payment-marker-pubkey.
import { writeFileSync } from "node:fs";
import { pubkeyHexFromJwk } from "./marker.js";

const out = process.env.OUT ?? "marker-key.jwk";
const { privateKey } = await crypto.subtle.generateKey({ name: "ECDSA", namedCurve: "P-256" }, true, ["sign", "verify"]);
const jwk = await crypto.subtle.exportKey("jwk", privateKey);
writeFileSync(out, JSON.stringify(jwk), { mode: 0o600 });
console.log(`private key written to ${out}`);
console.log(`pubkey hex: ${pubkeyHexFromJwk(jwk)}`);
