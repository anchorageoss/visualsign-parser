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
