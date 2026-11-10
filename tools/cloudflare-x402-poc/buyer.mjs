// x402 v2 buyer for the Cloudflare Worker PoC, Base Sepolia only.
//
// Reuses the #304 buyer flow (@x402/evm exact client). The wallet key is read
// from disk and never printed. A request/response transcript (no key
// material) is written to TRANSCRIPT, mode 0600, with the PAYMENT-SIGNATURE
// header redacted: it is a live EIP-3009 authorization until settled or
// expired.

import { chmod, readFile, writeFile } from "node:fs/promises";
import { createPublicClient, http, parseAbi } from "viem";
import { privateKeyToAccount } from "viem/accounts";
import { baseSepolia } from "viem/chains";
import { x402Client, x402HTTPClient } from "@x402/core/client";
import { registerExactEvmScheme } from "@x402/evm/exact/client";

const WORKER_URL = process.env.WORKER_URL;
if (!WORKER_URL) {
  console.error("set WORKER_URL to the deployed Worker, e.g. https://vsp-x402-poc.<subdomain>.workers.dev");
  process.exit(1);
}
const RPC_URL = process.env.RPC_URL ?? "https://sepolia.base.org";
const WALLET_KEY_PATH =
  process.env.WALLET_KEY ?? `${process.env.HOME}/.config/visualsign/base-sepolia/wallet.key`;
const TRANSCRIPT = process.env.TRANSCRIPT ?? "transcript.json";
const USDC_BASE_SEPOLIA = "0x036CbD53842c5426634e7929541eC2318f3dCF7e";

const transcript = [];

function redactHeaders(headers = {}) {
  return Object.fromEntries(
    Object.entries(headers).map(([k, v]) => [k, k.toLowerCase() === "payment-signature" ? "<redacted>" : v]),
  );
}

async function call(label, url, init) {
  const res = await fetch(url, init);
  const text = await res.text();
  transcript.push({
    label,
    request: { method: init.method, url, headers: redactHeaders(init.headers), body: init.body },
    response: {
      status: res.status,
      headers: Object.fromEntries(res.headers.entries()),
      body: text,
    },
  });
  return { res, text };
}

const key = (await readFile(WALLET_KEY_PATH, "utf-8")).trim();
const account = privateKeyToAccount(`0x${key.replace(/^0x/, "")}`);
console.log("buyer address:", account.address);

const pub = createPublicClient({ chain: baseSepolia, transport: http(RPC_URL) });
const usdc = await pub.readContract({
  address: USDC_BASE_SEPOLIA,
  abi: parseAbi(["function balanceOf(address) view returns (uint256)"]),
  functionName: "balanceOf",
  args: [account.address],
});
console.log("buyer USDC   :", Number(usdc) / 1e6);
if (usdc < 1000n) {
  console.error("USDC balance below the 0.001 price; fund via https://faucet.circle.com (Base Sepolia).");
  process.exit(1);
}

const core = new x402Client();
registerExactEvmScheme(core, { signer: account });
const httpClient = new x402HTTPClient(core);

const url = `${WORKER_URL}/visualsign/api/v2/parse`;
const ethTx =
  "0xec808504a817c800825208943535353535353535353535353535353535353535880de0b6b3a764000080018080";
const base = {
  method: "POST",
  headers: { "content-type": "application/json" },
  body: JSON.stringify({ request: { unsigned_payload: ethTx, chain: "CHAIN_ETHEREUM" } }),
};

const first = await call("1. unpaid request", url, base);
console.log("unpaid status:", first.res.status);
if (first.res.status !== 402) process.exit(1);

const required = httpClient.getPaymentRequiredResponse(
  (name) => first.res.headers.get(name),
  JSON.parse(first.text),
);
const payload = await httpClient.createPaymentPayload(required);
const paymentHeaders = httpClient.encodePaymentSignatureHeader(payload);

const second = await call("2. paid request", url, {
  ...base,
  headers: { ...base.headers, ...paymentHeaders },
});
console.log("paid status  :", second.res.status);

const receipt = second.res.headers.get("payment-response");
if (receipt) {
  const decoded = JSON.parse(Buffer.from(receipt, "base64").toString());
  console.log("settlement   :", JSON.stringify(decoded));
  if (decoded.transaction) console.log("explorer     :", `https://sepolia.basescan.org/tx/${decoded.transaction}`);
}

await writeFile(TRANSCRIPT, JSON.stringify(transcript, null, 2), { mode: 0o600 });
// `mode` only applies when the file is created; tighten an existing one too.
await chmod(TRANSCRIPT, 0o600);
console.log("transcript   :", TRANSCRIPT);
process.exit(second.res.status === 200 ? 0 : 1);
