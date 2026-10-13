// x402 v2 buyer for the Cloudflare Worker PoC, Base Sepolia only.
//
// Reuses the #304 buyer flow (@x402/evm exact client). Transport goes through
// an optional SOCKS5 proxy (SOCKS_PROXY=host:port) because Cloudflare is not
// directly reachable from the dev box. The wallet key is read from disk and
// never printed. A request/response transcript (no key material) is written
// to TRANSCRIPT for posting.

import { readFile, writeFile } from "node:fs/promises";
import { fetch as undiciFetch } from "undici";
import { socksDispatcher } from "fetch-socks";
import { createPublicClient, http, parseAbi } from "viem";
import { privateKeyToAccount } from "viem/accounts";
import { baseSepolia } from "viem/chains";
import { x402Client, x402HTTPClient } from "@x402/core/client";
import { registerExactEvmScheme } from "@x402/evm/exact/client";

const WORKER_URL = process.env.WORKER_URL ?? "https://vsp-x402-poc.anchorage-sandbox.workers.dev";
const RPC_URL = process.env.RPC_URL ?? "https://sepolia.base.org";
const WALLET_KEY_PATH =
  process.env.WALLET_KEY ?? `${process.env.HOME}/.config/visualsign/base-sepolia/wallet.key`;
const TRANSCRIPT = process.env.TRANSCRIPT ?? "transcript.json";
const USDC_BASE_SEPOLIA = "0x036CbD53842c5426634e7929541eC2318f3dCF7e";

let dispatcher;
if (process.env.SOCKS_PROXY) {
  const [host, port] = process.env.SOCKS_PROXY.split(":");
  dispatcher = socksDispatcher({ type: 5, host, port: Number(port) });
}

const transcript = [];

async function call(label, url, init) {
  const res = await undiciFetch(url, { ...init, dispatcher });
  const text = await res.text();
  transcript.push({
    label,
    request: { method: init.method, url, headers: init.headers, body: init.body },
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
  "0xf86c808504a817c800825208943535353535353535353535353535353535353535880de0b6b3a76400008025a028ef61340bd939bc2195fe537567866003e1a15d3c71ff63e1590620aa636276a067cbe9d8997f761aecb703304b3800ccf555c9f3dc64214b297fb1966a3b6d83";
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

await writeFile(TRANSCRIPT, JSON.stringify(transcript, null, 2));
console.log("transcript   :", TRANSCRIPT);
process.exit(second.res.status === 200 ? 0 : 1);
