// x402 v2 paywall for POST /visualsign/api/v2/parse, as a Cloudflare Worker.
//
// Testnet only: the advertised network is hardcoded to Base Sepolia and the
// worker refuses any payment that does not match the requirements it issued.
// Flow mirrors parser_gateway: verify -> forward -> settle only on a 2xx.

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

async function facilitator(env, path, body) {
  const base = env.FACILITATOR_URL ?? "https://x402.org/facilitator";
  const res = await fetch(`${base}${path}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
    signal: AbortSignal.timeout(FACILITATOR_TIMEOUT_MS),
  });
  const text = await res.text();
  try {
    return { status: res.status, body: JSON.parse(text) };
  } catch {
    return { status: res.status, body: { raw: text.slice(0, 200) } };
  }
}

async function runParse(env, request, bodyText) {
  if (!env.PARSER_BACKEND_URL) {
    // No backend wired: prove the payment flow with a clearly-labelled stub.
    return new Response(
      JSON.stringify({ stub: true, note: "PARSER_BACKEND_URL unset", bytes: bodyText.length }),
      { status: 200, headers: { "content-type": "application/json" } },
    );
  }
  const headers = { "content-type": "application/json" };
  if (env.BACKEND_BEARER) headers.authorization = `Bearer ${env.BACKEND_BEARER}`;
  // The backend's own v2 route is x402-gated; the Worker is the paywall, so it
  // calls the open v1 route (protect the backend with a bearer token).
  return fetch(`${env.PARSER_BACKEND_URL}${BACKEND_PATH}`, {
    method: "POST",
    headers,
    body: bodyText,
  });
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

  const bodyText = await request.text();
  if (enc.encode(bodyText).length > MAX_BODY_BYTES) {
    return json(413, { error: "body too large" });
  }

  const fbody = { x402Version: 2, paymentPayload: payload, paymentRequirements: want };

  const verify = await facilitator(env, "/verify", fbody);
  if (verify.status !== 200 || verify.body.isValid !== true) {
    return paymentRequired(env, resourceUrl, `verify failed: ${verify.body.invalidReason ?? verify.status}`);
  }

  const upstream = await runParse(env, request, bodyText);
  if (upstream.status < 200 || upstream.status >= 300) {
    // Not settled: an unserved request is never charged.
    return json(502, { error: "backend failed; payment not settled", backendStatus: upstream.status });
  }

  const settle = await facilitator(env, "/settle", fbody);
  if (settle.status !== 200 || settle.body.success !== true) {
    return json(502, { error: "settlement failed", reason: settle.body.errorReason ?? settle.status });
  }

  const out = new Response(upstream.body, upstream);
  out.headers.set("PAYMENT-RESPONSE", b64json(settle.body));
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
