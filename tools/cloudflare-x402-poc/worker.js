// x402 v2 paywall for POST /visualsign/api/v2/parse, as a Cloudflare Worker.
//
// Testnet only: the advertised network is hardcoded to Base Sepolia and the
// worker refuses any payment that does not match the requirements it issued.
// Flow mirrors parser_gateway: verify -> forward -> settle only on a 2xx.
//
// When MARKER_SIGNING_JWK is set the backend enforces proof of payment: the
// Worker settles first, then signs a marker bound to the request and the
// settlement (see marker.js) and sends it as X-Payment-Marker. A backend
// failure after settlement therefore cannot be un-charged.

import { CHAIN_NUMBERS, requestHash, sha256, signMarker } from "./marker.js";

const NETWORK = "eip155:84532";
const USDC_BASE_SEPOLIA = "0x036CbD53842c5426634e7929541eC2318f3dCF7e";
const PARSE_PATH = "/visualsign/api/v2/parse";
const BACKEND_PATH = "/visualsign/api/v1/parse";
const MAX_BODY_BYTES = 1024 * 1024;
const FACILITATOR_TIMEOUT_MS = 8000;

const enc = new TextEncoder();

function b64json(value) {
  const bytes = enc.encode(JSON.stringify(value));
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin);
}

function unb64json(s) {
  const bin = atob(s);
  const bytes = Uint8Array.from(bin, (c) => c.charCodeAt(0));
  return JSON.parse(new TextDecoder().decode(bytes));
}

function json(status, body, headers = {}) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json", ...headers },
  });
}

function requirements(env) {
  return {
    scheme: "exact",
    network: NETWORK,
    asset: USDC_BASE_SEPOLIA,
    amount: env.PRICE_ATOMIC ?? "1000",
    payTo: env.PAY_TO,
    maxTimeoutSeconds: 300,
    extra: { name: "USDC", version: "2" },
  };
}

function paymentRequired(env, url, error) {
  const body = {
    x402Version: 2,
    ...(error ? { error } : {}),
    resource: {
      url,
      description: "VisualSign parse",
      mimeType: "application/json",
    },
    accepts: [requirements(env)],
  };
  return json(402, body, { "PAYMENT-REQUIRED": b64json(body) });
}

// The client echoes the requirement it chose in `accepted`. Refuse anything
// that differs from what we advertised, so a client cannot underpay.
function acceptedMatches(accepted, want) {
  if (!accepted || typeof accepted !== "object") return false;
  return (
    accepted.scheme === want.scheme &&
    accepted.network === want.network &&
    String(accepted.asset).toLowerCase() === want.asset.toLowerCase() &&
    String(accepted.amount) === want.amount &&
    String(accepted.payTo).toLowerCase() === want.payTo.toLowerCase()
  );
}

// A transport failure (DNS, connect, timeout) returns `unreachable` set and
// status 0 instead of throwing, so callers can tell "the facilitator said no"
// from "we never got an answer".
async function facilitator(env, path, body) {
  const base = env.FACILITATOR_URL ?? "https://x402.org/facilitator";
  let res;
  let text;
  try {
    res = await fetch(`${base}${path}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
      signal: AbortSignal.timeout(FACILITATOR_TIMEOUT_MS),
    });
    text = await res.text();
  } catch (e) {
    return { status: 0, unreachable: e?.name === "TimeoutError" ? "timeout" : "unreachable", body: {} };
  }
  try {
    return { status: res.status, body: JSON.parse(text) };
  } catch {
    return { status: res.status, body: { raw: text.slice(0, 200) } };
  }
}

// Settlement either failed (the facilitator answered no) or its outcome is
// unknown (no answer). An unknown settle may still land on-chain. Retrying
// with the same PAYMENT-SIGNATURE cannot charge twice: the requirements set no
// `assetTransferMethod`, so @x402/evm pays with EIP-3009
// transferWithAuthorization, and USDC marks the authorization nonce used; a
// replay is refused (`invalid_exact_evm_nonce_already_used`) instead of
// settling again. The buyer resolves an unknown outcome by reading
// `authorizationState(payer, nonce)` on the USDC contract.
function settleError(settle, headers = {}) {
  if (settle.unreachable) {
    return json(
      502,
      { error: "settlement outcome unknown", settlement: "unknown", reason: settle.unreachable },
      headers,
    );
  }
  return json(
    502,
    { error: "settlement failed", settlement: "failed", reason: settle.body.errorReason ?? settle.status },
    headers,
  );
}

// Reads at most `max` bytes of the body, cancelling the stream once the limit
// is crossed, so an oversized body is never buffered whole. Returns null when
// the body is too large.
async function readBodyCapped(request, max) {
  const declared = Number(request.headers.get("content-length"));
  if (declared > max) return null;
  if (!request.body) return "";
  const reader = request.body.getReader();
  const chunks = [];
  let total = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    total += value.byteLength;
    if (total > max) {
      await reader.cancel();
      return null;
    }
    chunks.push(value);
  }
  const bytes = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return new TextDecoder().decode(bytes);
}

// The fields the marker binds. chain_metadata is unsupported (see marker.js).
function markerRequest(bodyText) {
  let req;
  try {
    req = JSON.parse(bodyText)?.request;
  } catch {
    return null;
  }
  const chain = CHAIN_NUMBERS[req?.chain];
  if (!chain || typeof req.unsigned_payload !== "string" || req.chain_metadata != null) return null;
  return { chain, unsignedPayload: req.unsigned_payload, includeIntermediate: req.include_intermediate_output === true };
}

async function runParse(env, request, bodyText, marker) {
  if (!env.PARSER_BACKEND_URL) {
    // No backend wired: prove the payment flow with a clearly-labelled stub.
    return new Response(
      JSON.stringify({ stub: true, note: "PARSER_BACKEND_URL unset", bytes: bodyText.length }),
      { status: 200, headers: { "content-type": "application/json" } },
    );
  }
  const headers = { "content-type": "application/json" };
  if (env.BACKEND_BEARER) headers.authorization = `Bearer ${env.BACKEND_BEARER}`;
  if (marker) headers["x-payment-marker"] = marker;
  // The backend's own v2 route is x402-gated; the Worker is the paywall, so it
  // calls the open v1 route (protect the backend with a bearer token).
  try {
    return await fetch(`${env.PARSER_BACKEND_URL}${BACKEND_PATH}`, {
      method: "POST",
      headers,
      body: bodyText,
    });
  } catch {
    // Unreachable backend: treated like a 5xx (no settle, or receipt returned).
    return new Response(null, { status: 502 });
  }
}

async function handleParse(request, env) {
  if (!env.PAY_TO) return json(500, { error: "PAY_TO not configured" });
  const url = new URL(request.url);
  const resourceUrl = `${url.origin}${PARSE_PATH}`;

  const sig = request.headers.get("PAYMENT-SIGNATURE");
  if (!sig) return paymentRequired(env, resourceUrl);

  let payload;
  try {
    payload = unb64json(sig);
  } catch {
    return paymentRequired(env, resourceUrl, "malformed PAYMENT-SIGNATURE");
  }
  const want = requirements(env);
  if (payload.x402Version !== 2 || !acceptedMatches(payload.accepted, want)) {
    return paymentRequired(env, resourceUrl, "payment does not match requirements");
  }

  const bodyText = await readBodyCapped(request, MAX_BODY_BYTES);
  if (bodyText === null) {
    return json(413, { error: "body too large" });
  }

  const signing = Boolean(env.MARKER_SIGNING_JWK);
  const mreq = signing ? markerRequest(bodyText) : null;
  if (signing && !mreq) {
    return json(400, { error: "body must carry request.chain (Ethereum/Solana), request.unsigned_payload and no chain_metadata" });
  }

  const fbody = { x402Version: 2, paymentPayload: payload, paymentRequirements: want };

  const verify = await facilitator(env, "/verify", fbody);
  if (verify.unreachable) {
    // Nothing verified, nothing settled: safe for the buyer to retry.
    return json(503, { error: "facilitator unavailable", stage: "verify", reason: verify.unreachable });
  }
  if (verify.status !== 200 || verify.body.isValid !== true) {
    return paymentRequired(env, resourceUrl, `verify failed: ${verify.body.invalidReason ?? verify.status}`);
  }

  if (!signing) {
    const upstream = await runParse(env, request, bodyText);
    if (upstream.status < 200 || upstream.status >= 300) {
      // Not settled: an unserved request is never charged.
      return json(502, { error: "backend failed; payment not settled", backendStatus: upstream.status });
    }
    const settle = await facilitator(env, "/settle", fbody);
    if (settle.status !== 200 || settle.body.success !== true) return settleError(settle);
    const out = new Response(upstream.body, upstream);
    out.headers.set("PAYMENT-RESPONSE", b64json(settle.body));
    return out;
  }

  const settle = await facilitator(env, "/settle", fbody);
  if (settle.status !== 200 || settle.body.success !== true) return settleError(settle);
  const receipt = b64json(settle.body);
  const marker = await signMarker(env.MARKER_SIGNING_JWK, {
    requestHash: await requestHash(mreq.chain, mreq.unsignedPayload, mreq.includeIntermediate),
    txid: settle.body.transaction,
    payer: settle.body.payer ?? "",
    payTo: want.payTo,
    amount: want.amount,
    mint: want.asset,
    xPaymentHash: await sha256(Uint8Array.from(atob(sig), (c) => c.charCodeAt(0))),
    network: NETWORK,
    settledAtMs: Date.now(),
  });
  const upstream = await runParse(env, request, bodyText, marker);
  if (upstream.status < 200 || upstream.status >= 300) {
    // Already settled: hand back the receipt so the buyer can see the charge.
    return json(
      502,
      { error: "backend failed after settlement", backendStatus: upstream.status },
      { "PAYMENT-RESPONSE": receipt },
    );
  }
  const out = new Response(upstream.body, upstream);
  out.headers.set("PAYMENT-RESPONSE", receipt);
  return out;
}

export default {
  async fetch(request, env) {
    const { pathname } = new URL(request.url);
    if (pathname === "/health") return json(200, { ok: true, network: NETWORK });
    if (pathname === PARSE_PATH) {
      if (request.method !== "POST") return json(405, { error: "POST only" });
      return handleParse(request, env);
    }
    return json(404, { error: "not found" });
  },
};
