// Prove a backend started with --require-payment-marker-pubkey enforces proof
// of payment, with no Worker or settlement involved: sign markers locally with
// the same code the Worker uses and call the backend's v1 parse route.
//
//   BACKEND_URL=http://localhost:8080 MARKER_KEY=marker-key.jwk node check-backend.mjs
//
// Expects: no marker 402, forged key 402, marker for another request 402,
// valid marker 200. Exits non-zero on any other outcome.
import { readFileSync } from "node:fs";
import { CHAIN_NUMBERS, requestHash, sha256, signMarker } from "./marker.js";

const base = process.env.BACKEND_URL;
if (!base) throw new Error("set BACKEND_URL");
const keyJson = readFileSync(process.env.MARKER_KEY ?? "marker-key.jwk", "utf8");
const headersExtra = process.env.BACKEND_BEARER ? { authorization: `Bearer ${process.env.BACKEND_BEARER}` } : {};

const TX =
  "0xec808504a817c800825208943535353535353535353535353535353535353535880de0b6b3a764000080018080";
const body = JSON.stringify({ request: { chain: "CHAIN_ETHEREUM", unsigned_payload: TX } });

async function marker(jwkJson, payload) {
  return signMarker(jwkJson, {
    requestHash: await requestHash(CHAIN_NUMBERS.CHAIN_ETHEREUM, payload, false),
    txid: "0xcheck",
    payer: "0xpayer",
    payTo: "0xpayee",
    amount: "1000",
    mint: "0xusdc",
    xPaymentHash: await sha256(new Uint8Array()),
    network: "eip155:84532",
    settledAtMs: Date.now(),
  });
}

async function call(label, want, markerValue) {
  const res = await fetch(`${base}/visualsign/api/v1/parse`, {
    method: "POST",
    headers: { "content-type": "application/json", ...headersExtra, ...(markerValue ? { "x-payment-marker": markerValue } : {}) },
    body,
  });
  await res.arrayBuffer();
  const ok = res.status === want;
  console.log(`${ok ? "ok  " : "FAIL"} ${label}: ${res.status} (want ${want})`);
  return ok;
}

const forged = await crypto.subtle.generateKey({ name: "ECDSA", namedCurve: "P-256" }, true, ["sign"]);
const forgedJwk = JSON.stringify(await crypto.subtle.exportKey("jwk", forged.privateKey));

const results = [
  await call("no marker", 402),
  await call("marker signed by an unpinned key", 402, await marker(forgedJwk, TX)),
  await call("valid marker bound to a different request", 402, await marker(keyJson, "0x02")),
  await call("valid marker", 200, await marker(keyJson, TX)),
];
process.exit(results.every(Boolean) ? 0 : 1);
