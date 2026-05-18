#!/usr/bin/env -S npx tsx
/**
 * x402 v2 demo client against visualsign-parser's gateway with payai
 * facilitator on Base Sepolia.
 *
 * Mirrors `x402-solana-devnet-demo.ts` but on EVM. Drives a real on-chain
 * USDC settlement: the buyer signs an EIP-3009 transferWithAuthorization
 * for Base Sepolia USDC, the gateway forwards to payai's facilitator,
 * payai settles on chain, the gateway signs a VPM, the TVC pivot verifies
 * the VPM + returns a signed parse.
 *
 * Prereqs:
 *   1. Gateway is running locally with `X402_PROFILE=payai`,
 *      `X402_NETWORK=base-sepolia`, `GATEWAY_SIGNING_KEY_FILE` set,
 *      `HTTP_BACKEND_URL` pointing at the TVC pivot, and `X402_PAYTO`
 *      pointing at the receiver (default: the buyer wallet, so the
 *      transfer is a self-transfer — same pattern as the Solana demo).
 *   2. The Base Sepolia wallet at ~/.config/visualsign/base-sepolia/wallet.key
 *      is funded with Base Sepolia USDC (Circle faucet, mint
 *      0x036CbD53842c5426634e7929541eC2318f3dCF7e). ETH for gas is
 *      not strictly required — payai's facilitator pays gas in the
 *      x402 v2 flow — but a little is helpful as a fallback.
 *
 * Env vars (all optional):
 *   GATEWAY_URL   default http://127.0.0.1:8080
 *   RPC_URL       default https://sepolia.base.org
 *   WALLET_KEY    default ~/.config/visualsign/base-sepolia/wallet.key
 */

import { readFile } from "node:fs/promises";
import { createPublicClient, http, parseAbi } from "viem";
import { privateKeyToAccount } from "viem/accounts";
import { baseSepolia } from "viem/chains";
import { x402Client, x402HTTPClient } from "@x402/core/client";
import { registerExactEvmScheme } from "@x402/evm/exact/client";

const GATEWAY_URL = process.env.GATEWAY_URL ?? "http://127.0.0.1:8080";
const RPC_URL = process.env.RPC_URL ?? "https://sepolia.base.org";
const WALLET_KEY_PATH =
  process.env.WALLET_KEY ?? `${process.env.HOME}/.config/visualsign/base-sepolia/wallet.key`;

const USDC_BASE_SEPOLIA = "0x036CbD53842c5426634e7929541eC2318f3dCF7e" as const;

function logSection(title: string): void {
  console.log("");
  console.log(`-- ${title} `.padEnd(72, "-"));
}

async function loadKey(): Promise<string> {
  try {
    return (await readFile(WALLET_KEY_PATH, "utf-8")).trim();
  } catch (e) {
    if ((e as NodeJS.ErrnoException).code === "ENOENT") {
      console.error(
        `wallet key not found at ${WALLET_KEY_PATH}\n` +
          `  Generate one with viem's generatePrivateKey, write 32-byte hex to that path (mode 600),\n` +
          `  then fund the derived address with Base Sepolia USDC (https://faucet.circle.com).`,
      );
      process.exit(1);
    }
    throw e;
  }
}

async function main(): Promise<void> {
  // ── Wallet ──────────────────────────────────────────────────────────
  const keyHex = await loadKey();
  const account = privateKeyToAccount(`0x${keyHex.replace(/^0x/, "")}`);
  logSection("Wallet");
  console.log("buyer address :", account.address);

  const pub = createPublicClient({ chain: baseSepolia, transport: http(RPC_URL) });
  const usdcAbi = parseAbi(["function balanceOf(address) view returns (uint256)"]);
  // ETH balance is informational only — payai's facilitator pays gas in
  // the x402 v2 flow. USDC is what actually matters. Fetch in parallel.
  const [ethBalance, usdcBalance] = await Promise.all([
    pub.getBalance({ address: account.address }),
    pub.readContract({
      address: USDC_BASE_SEPOLIA,
      abi: usdcAbi,
      functionName: "balanceOf",
      args: [account.address],
    }),
  ]);
  console.log("buyer ETH     :", (Number(ethBalance) / 1e18).toFixed(6), "ETH (informational; payai pays gas)");
  console.log("buyer USDC    :", (Number(usdcBalance) / 1e6).toFixed(6), "USDC");
  if (usdcBalance < 100n) {
    console.error(
      "ERROR: USDC balance too low to pay the 402 (need ≥ 0.0001 USDC).\n" +
        "  Top up at https://faucet.circle.com — pick Base Sepolia, mint 0x036CbD…CF7e.",
    );
    process.exit(1);
  }

  // ── x402 client ─────────────────────────────────────────────────────
  logSection("x402 client (@x402/evm exact)");
  const core = new x402Client();
  registerExactEvmScheme(core, { signer: account });
  const httpClient = new x402HTTPClient(core);
  console.log("client constructed; making initial request…");

  const url = `${GATEWAY_URL}/visualsign/api/v2/parse`;
  // Same EIP-1559 ETH tx used in every other smoke test.
  const ethTx =
    "0xf86c808504a817c800825208943535353535353535353535353535353535353535880de0b6b3a76400008025a028ef61340bd939bc2195fe537567866003e1a15d3c71ff63e1590620aa636276a067cbe9d8997f761aecb703304b3800ccf555c9f3dc64214b297fb1966a3b6d83";
  const body = JSON.stringify({
    request: { unsigned_payload: ethTx, chain: "CHAIN_ETHEREUM" },
  });
  const requestInit = {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body,
  };

  // ── Step 1: unpaid → 402 ────────────────────────────────────────────
  logSection("Unpaid POST → 402");
  let resp = await fetch(url, requestInit);
  console.log("status:", resp.status);
  if (resp.status !== 402) {
    console.error("expected 402; got:", resp.status, await resp.text());
    process.exit(1);
  }
  const paymentRequired = httpClient.getPaymentRequiredResponse(
    (name) => resp.headers.get(name),
    await resp.json().catch(() => undefined),
  );
  console.log("requirements:", JSON.stringify(paymentRequired.accepts[0], null, 2));

  // ── Step 2: build + sign payload ────────────────────────────────────
  logSection("Sign + encode payment");
  const paymentPayload = await httpClient.createPaymentPayload(paymentRequired);
  const paymentHeaders = httpClient.encodePaymentSignatureHeader(paymentPayload);
  console.log(
    "encoded header keys:",
    Object.keys(paymentHeaders).join(", "),
  );

  // ── Step 3: paid request ────────────────────────────────────────────
  logSection("Paid POST /visualsign/api/v2/parse");
  resp = await fetch(url, {
    ...requestInit,
    headers: { ...requestInit.headers, ...paymentHeaders },
  });
  console.log("status:", resp.status);
  const respBody: {
    response?: {
      parsedTransaction?: {
        payload?: { signablePayload?: string };
        signature?: { publicKey?: string };
      };
    };
    error?: string | null;
  } = await resp.json();
  if (resp.status !== 200) {
    console.error("paid request failed:", JSON.stringify(respBody, null, 2));
    process.exit(1);
  }
  console.log("response.error :", respBody.error ?? null);
  console.log(
    "payloadLen     :",
    (respBody.response?.parsedTransaction?.payload?.signablePayload ?? "").length,
  );
  console.log(
    "enclavePubKey  :",
    respBody.response?.parsedTransaction?.signature?.publicKey ?? "(missing)",
  );

  // ── Settlement receipt ──────────────────────────────────────────────
  const paymentResponse = resp.headers.get("Payment-Response");
  if (paymentResponse) {
    logSection("Settlement (Payment-Response)");
    const decoded = JSON.parse(Buffer.from(paymentResponse, "base64").toString());
    console.log(JSON.stringify(decoded, null, 2));
    if (decoded.transaction && typeof decoded.transaction === "string") {
      console.log(
        "explorer       :",
        `https://sepolia.basescan.org/tx/${decoded.transaction}`,
      );
    }
  }

  logSection("Done");
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
