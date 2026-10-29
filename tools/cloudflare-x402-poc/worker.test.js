import { test } from "node:test";
import assert from "node:assert/strict";
import worker from "./worker.js";

const PAY_TO = "0x000000000000000000000000000000000000dEaD";
const env = { PAY_TO };
const USDC = "0x036CbD53842c5426634e7929541eC2318f3dCF7e";

function accepted(over = {}) {
  return {
    scheme: "exact",
    network: "eip155:84532",
    asset: USDC,
    amount: "1000",
    payTo: PAY_TO,
    maxTimeoutSeconds: 300,
    extra: { name: "USDC", version: "2" },
    ...over,
  };
}

function sigHeader(over) {
  const payload = { x402Version: 2, accepted: accepted(over), payload: {} };
  return btoa(JSON.stringify(payload));
}

function req(headers = {}) {
  return new Request("https://w.example/visualsign/api/v2/parse", {
    method: "POST",
    headers,
    body: "{}",
  });
}

function mockFacilitator({ valid = true, settle = true } = {}) {
  const calls = [];
  globalThis.fetch = async (url, init) => {
    calls.push(String(url));
    if (String(url).endsWith("/verify")) {
      return new Response(JSON.stringify({ isValid: valid, invalidReason: "bad" }));
    }
    return new Response(JSON.stringify({ success: settle, transaction: "0xabc", network: "eip155:84532" }));
  };
  return calls;
}

test("no signature -> 402 with PAYMENT-REQUIRED on base sepolia", async () => {
  const res = await worker.fetch(req(), env);
  assert.equal(res.status, 402);
  const body = JSON.parse(atob(res.headers.get("PAYMENT-REQUIRED")));
  assert.equal(body.accepts[0].network, "eip155:84532");
  assert.equal(body.accepts[0].amount, "1000");
});

test("underpayment is rejected before the facilitator is called", async () => {
  const calls = mockFacilitator();
  const res = await worker.fetch(req({ "PAYMENT-SIGNATURE": sigHeader({ amount: "1" }) }), env);
  assert.equal(res.status, 402);
  assert.equal(calls.length, 0);
});

test("mainnet network is rejected", async () => {
  const calls = mockFacilitator();
  const res = await worker.fetch(req({ "PAYMENT-SIGNATURE": sigHeader({ network: "eip155:8453" }) }), env);
  assert.equal(res.status, 402);
  assert.equal(calls.length, 0);
});

test("valid payment -> verify, serve, settle, PAYMENT-RESPONSE", async () => {
  const calls = mockFacilitator();
  const res = await worker.fetch(req({ "PAYMENT-SIGNATURE": sigHeader() }), env);
  assert.equal(res.status, 200);
  assert.deepEqual(calls.map((c) => c.split("/").pop()), ["verify", "settle"]);
  assert.equal(JSON.parse(atob(res.headers.get("PAYMENT-RESPONSE"))).transaction, "0xabc");
});

test("verify failure -> 402, no settle", async () => {
  const calls = mockFacilitator({ valid: false });
  const res = await worker.fetch(req({ "PAYMENT-SIGNATURE": sigHeader() }), env);
  assert.equal(res.status, 402);
  assert.deepEqual(calls.map((c) => c.split("/").pop()), ["verify"]);
});

test("backend failure -> 502 and no settle", async () => {
  const calls = [];
  globalThis.fetch = async (url) => {
    calls.push(String(url));
    if (String(url).endsWith("/verify")) return new Response(JSON.stringify({ isValid: true }));
    if (String(url).includes("backend.example")) return new Response("boom", { status: 500 });
    return new Response(JSON.stringify({ success: true }));
  };
  const res = await worker.fetch(
    req({ "PAYMENT-SIGNATURE": sigHeader() }),
    { ...env, PARSER_BACKEND_URL: "https://backend.example" },
  );
  assert.equal(res.status, 502);
  assert.ok(!calls.some((c) => c.endsWith("/settle")));
});

// Proof-of-payment mode: MARKER_SIGNING_JWK set, so settle happens first and
// the backend receives a signed marker.
async function markerEnv() {
  const { privateKey } = await crypto.subtle.generateKey({ name: "ECDSA", namedCurve: "P-256" }, true, ["sign"]);
  const jwk = JSON.stringify(await crypto.subtle.exportKey("jwk", privateKey));
  return { PAY_TO, PARSER_BACKEND_URL: "https://backend.example", MARKER_SIGNING_JWK: jwk };
}

const PARSE_BODY = JSON.stringify({ request: { chain: "CHAIN_ETHEREUM", unsigned_payload: "0x02" } });

function paidReq(body = PARSE_BODY) {
  return new Request("https://w.example/visualsign/api/v2/parse", {
    method: "POST",
    headers: { "PAYMENT-SIGNATURE": sigHeader() },
    body,
  });
}

function mockWithBackend(backendStatus) {
  const calls = [];
  globalThis.fetch = async (url, init) => {
    const u = String(url);
    calls.push({ url: u, headers: init?.headers });
    if (u.endsWith("/verify")) return new Response(JSON.stringify({ isValid: true }));
    if (u.endsWith("/settle")) {
      return new Response(JSON.stringify({ success: true, transaction: "0xabc", payer: "0xpayer", network: "eip155:84532" }));
    }
    return new Response("{}", { status: backendStatus });
  };
  return calls;
}

test("marker mode settles before forwarding and sends a marker", async () => {
  const calls = mockWithBackend(200);
  const res = await worker.fetch(paidReq(), await markerEnv());
  assert.equal(res.status, 200);
  assert.deepEqual(
    calls.map((c) => c.url.split("/").pop()),
    ["verify", "settle", "parse"],
  );
  const marker = calls[2].headers["x-payment-marker"];
  assert.ok(marker && marker.length > 100);
  assert.ok(res.headers.get("PAYMENT-RESPONSE"));
});

test("marker mode: backend failure after settlement still returns the receipt", async () => {
  mockWithBackend(500);
  const res = await worker.fetch(paidReq(), await markerEnv());
  assert.equal(res.status, 502);
  assert.ok(res.headers.get("PAYMENT-RESPONSE"));
});

test("marker mode refuses a body it cannot bind before calling the facilitator", async () => {
  const calls = mockWithBackend(200);
  const withMetadata = JSON.stringify({
    request: { chain: "CHAIN_ETHEREUM", unsigned_payload: "0x02", chain_metadata: { ethereum: {} } },
  });
  const res = await worker.fetch(paidReq(withMetadata), await markerEnv());
  assert.equal(res.status, 400);
  assert.equal(calls.length, 0);
});

test("oversized declared body -> 413 before the facilitator is called", async () => {
  const calls = mockFacilitator();
  const res = await worker.fetch(
    new Request("https://w.example/visualsign/api/v2/parse", {
      method: "POST",
      headers: { "PAYMENT-SIGNATURE": sigHeader(), "content-length": String(2 * 1024 * 1024) },
      body: "{}",
    }),
    env,
  );
  assert.equal(res.status, 413);
  assert.equal(calls.length, 0);
});

test("oversized streamed body -> 413, stream cancelled", async () => {
  const calls = mockFacilitator();
  let pulled = 0;
  let cancelled = false;
  const chunk = new Uint8Array(256 * 1024);
  const body = new ReadableStream({
    pull(controller) {
      pulled += 1;
      controller.enqueue(chunk);
    },
    cancel() {
      cancelled = true;
    },
  });
  const res = await worker.fetch(
    new Request("https://w.example/visualsign/api/v2/parse", {
      method: "POST",
      headers: { "PAYMENT-SIGNATURE": sigHeader() },
      body,
      duplex: "half",
    }),
    env,
  );
  assert.equal(res.status, 413);
  assert.ok(cancelled);
  assert.ok(pulled <= 6, `read ${pulled} chunks`);
  assert.equal(calls.length, 0);
});

function mockUnreachable(stage) {
  const calls = [];
  globalThis.fetch = async (url) => {
    const u = String(url);
    calls.push(u);
    if (u.endsWith(`/${stage}`)) throw new DOMException("timed out", "TimeoutError");
    if (u.endsWith("/verify")) return new Response(JSON.stringify({ isValid: true }));
    if (u.endsWith("/settle")) return new Response(JSON.stringify({ success: true, transaction: "0xabc" }));
    return new Response("{}", { status: 200 });
  };
  return calls;
}

test("verify timeout -> 503, no settle", async () => {
  const calls = mockUnreachable("verify");
  const res = await worker.fetch(req({ "PAYMENT-SIGNATURE": sigHeader() }), env);
  assert.equal(res.status, 503);
  assert.equal((await res.json()).reason, "timeout");
  assert.ok(!calls.some((c) => c.endsWith("/settle")));
});

test("settle timeout -> 502 with settlement outcome unknown", async () => {
  mockUnreachable("settle");
  const res = await worker.fetch(req({ "PAYMENT-SIGNATURE": sigHeader() }), env);
  assert.equal(res.status, 502);
  assert.equal((await res.json()).settlement, "unknown");
});

test("marker mode: settle timeout -> 502 unknown, backend not called", async () => {
  const calls = mockUnreachable("settle");
  const res = await worker.fetch(paidReq(), await markerEnv());
  assert.equal(res.status, 502);
  assert.equal((await res.json()).settlement, "unknown");
  assert.ok(!calls.some((c) => c.includes("backend.example")));
});

test("marker mode: unreachable backend after settlement still returns the receipt", async () => {
  globalThis.fetch = async (url) => {
    const u = String(url);
    if (u.endsWith("/verify")) return new Response(JSON.stringify({ isValid: true }));
    if (u.endsWith("/settle")) return new Response(JSON.stringify({ success: true, transaction: "0xabc", payer: "0xp" }));
    throw new TypeError("fetch failed");
  };
  const res = await worker.fetch(paidReq(), await markerEnv());
  assert.equal(res.status, 502);
  assert.ok(res.headers.get("PAYMENT-RESPONSE"));
});
