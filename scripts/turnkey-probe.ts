#!/usr/bin/env -S node --experimental-strip-types --no-warnings
/**
 * One-off probe: send an X-Stamp-authenticated request to a Turnkey TVC app
 * URL and dump the response. Uses the API key at
 * `~/.config/turnkey/keys/<name>.{private,public}`.
 *
 * Usage: turnkey-probe.ts <APP_URL> <PATH> [JSON_BODY]
 *
 * Example:
 *   turnkey-probe.ts https://app-e34...turnkey.cloud /visualsign/api/v1/parse \
 *     '{"request":{"unsigned_payload":"0xdeadbeef","chain":"CHAIN_ETHEREUM"}}'
 *
 * Wire shape per visualsign-turnkey-client/main.go:310-332:
 *   X-Stamp = base64url(JSON {publicKey, signature, scheme: "SIGNATURE_SCHEME_TK_API_P256"})
 *   signature = DER(ECDSA-P256(SHA256(body)))
 */
import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { p256 } from "@noble/curves/p256";
import { sha256 } from "@noble/hashes/sha2";

const KEY_NAME = process.env.TURNKEY_KEY_NAME ?? "default";
const KEY_DIR = `${homedir()}/.config/turnkey/keys`;
const PRIV_PATH = `${KEY_DIR}/${KEY_NAME}.private`;
const PUB_PATH = `${KEY_DIR}/${KEY_NAME}.public`;

const [, , appUrl, pathArg, bodyArg] = process.argv;
if (!appUrl || !pathArg) {
  console.error("usage: turnkey-probe.ts <APP_URL> <PATH> [JSON_BODY]");
  process.exit(1);
}

const body = bodyArg ?? "";

function loadKey(): { privHex: string; pubHex: string } {
  // Go convention: <name>.private contains "<hex>:p256" or just "<hex>".
  const rawPriv = readFileSync(PRIV_PATH, "utf-8").trim();
  const privHex = rawPriv.split(":")[0];
  const pubHex = readFileSync(PUB_PATH, "utf-8").trim();
  return { privHex, pubHex };
}

const { privHex, pubHex } = loadKey();
const priv = Buffer.from(privHex, "hex");

const bodyBytes = Buffer.from(body, "utf-8");
const digest = sha256(bodyBytes);

// p256.sign(digest, priv) returns a compact (r||s) signature by default;
// we DER-encode it.
const sig = p256.sign(digest, priv, { lowS: true, prehash: false });
const derHex = Buffer.from(sig.toDERRawBytes()).toString("hex");

const stamp = {
  publicKey: pubHex,
  signature: derHex,
  scheme: "SIGNATURE_SCHEME_TK_API_P256",
};
const stampHeader = Buffer.from(JSON.stringify(stamp), "utf-8")
  .toString("base64")
  .replace(/\+/g, "-")
  .replace(/\//g, "_")
  .replace(/=+$/, "");

console.error(`POST ${appUrl}${pathArg}`);
console.error(`  X-Stamp: ${stampHeader.slice(0, 80)}...`);
console.error(`  body bytes: ${bodyBytes.length}`);
console.error("");

const resp = await fetch(`${appUrl}${pathArg}`, {
  method: body ? "POST" : "GET",
  headers: {
    "content-type": "application/json",
    "X-Stamp": stampHeader,
  },
  body: body || undefined,
});
console.log(`HTTP ${resp.status} ${resp.statusText}`);
for (const [k, v] of resp.headers) {
  console.log(`${k}: ${v}`);
}
console.log("");
const text = await resp.text();
console.log(text);
