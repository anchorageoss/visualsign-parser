#!/usr/bin/env -S npx tsx
/**
 * x402 demo client against the visualsign-parser gateway with payai facilitator
 * on Solana devnet.
 *
 * Drives the same flow as the gated Rust integration test
 * (`tests/x402_payai_devnet_test.rs`) but lives outside the cargo test loop so
 * humans can poke at it without rebuilding the workspace.
 *
 * Why this exists in TypeScript:
 *   payai's `x402-solana` npm package is the **supported** Solana x402 client.
 *   The repo's Rust client in `src/integration/src/solana_x402_client.rs` is
 *   test-only, and we deliberately do not ship a production Rust client.
 *
 * Prerequisites:
 *   1. The gateway is running locally with X402_PROFILE=payai and
 *      X402_NETWORK=solana-devnet. Easiest: `make dev-up-payai` from the repo
 *      root, which brings up parser_grpc_server + parser_gateway as stagex
 *      images via compose.payai.yml.
 *   2. The reproducible test wallet from `src/integration/fixtures/devnet/`
 *      is funded with devnet SOL (faucet.solana.com) and devnet USDC
 *      (faucet.circle.com, mint 4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU).
 *      Current address (from wallet.seed):
 *        x2iWww6XjauBk83HpBMzkGPijbzy4vqdRzS5skWPxmW
 *
 * Env vars (all optional):
 *   GATEWAY_URL  default http://127.0.0.1:8080
 *   RPC_URL      default https://api.devnet.solana.com
 *   WALLET_SEED  default ../src/integration/fixtures/devnet/wallet.seed
 *   X402_TVC_VERIFIER_PUBKEY_HEX
 *                if set, the demo independently P256-verifies the response
 *                signature against this key (cross-impl check vs the gateway).
 */

import { Connection, Keypair, PublicKey, VersionedTransaction } from "@solana/web3.js";
import { fileURLToPath } from "node:url";
import { readFile } from "node:fs/promises";
import { p256 } from "@noble/curves/p256";
import { dirname, resolve } from "node:path";

// PayAI's vetted Solana x402 client (TS only). If the package is not installed
// the script will fall back to a hand-built X-PAYMENT header.
let createPaymentHeader: ((opts: unknown) => Promise<string>) | null = null;
try {
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  const mod = await import("x402-solana");
  // The exact export name has drifted across payai SDK versions; pick whichever
  // is present.
  createPaymentHeader =
    (mod as Record<string, unknown>).createPaymentHeader as
      | ((opts: unknown) => Promise<string>)
      | null
    ?? null;
} catch {
  createPaymentHeader = null;
}

const GATEWAY_URL = process.env.GATEWAY_URL ?? "http://127.0.0.1:8080";
const RPC_URL = process.env.RPC_URL ?? "https://api.devnet.solana.com";
const TVC_HEX = process.env.X402_TVC_VERIFIER_PUBKEY_HEX?.toLowerCase();

async function loadBuyerKeypair(): Promise<Keypair> {
  const __filename = fileURLToPath(import.meta.url);
  const __dirname = dirname(__filename);
  const defaultSeedPath = resolve(
    __dirname,
    "../src/integration/fixtures/devnet/wallet.seed",
  );
  const seedPath = process.env.WALLET_SEED ?? defaultSeedPath;
  const raw = await readFile(seedPath, "utf-8");
  const seed = Buffer.from(raw.trim(), "utf-8");
  if (seed.length !== 32) {
    throw new Error(`wallet.seed must be 32 bytes; got ${seed.length}`);
  }
  return Keypair.fromSeed(seed);
}

function logSection(title: string): void {
  console.log("");
  console.log(`── ${title} `.padEnd(72, "─"));
}

async function main(): Promise<void> {
  const buyer = await loadBuyerKeypair();
  logSection("Wallet");
  console.log("buyer address :", buyer.publicKey.toBase58());
  const conn = new Connection(RPC_URL, "confirmed");
  const lamports = await conn.getBalance(buyer.publicKey, "confirmed");
  console.log("buyer balance :", (lamports / 1e9).toFixed(4), "SOL on devnet");
  if (lamports < 50_000_000) {
    console.warn(
      `WARNING: low SOL balance (${lamports} lamports). Run \`solana airdrop 2 ${buyer.publicKey.toBase58()} --url devnet\` or visit https://faucet.solana.com`,
    );
  }

  logSection("Probe 402");
  const probeBody = {
    request: { unsigned_payload: "0xdeadbeef", chain: "CHAIN_ETHEREUM" },
  };
  const probe = await fetch(`${GATEWAY_URL}/visualsign/api/v2/parse`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(probeBody),
  });
  if (probe.status !== 402) {
    throw new Error(`expected 402 from gateway, got ${probe.status}`);
  }
  const paymentRequiredB64 = probe.headers.get("Payment-Required");
  if (!paymentRequiredB64) {
    throw new Error("missing Payment-Required header on 402");
  }
  const paymentRequired = JSON.parse(
    Buffer.from(paymentRequiredB64, "base64").toString("utf-8"),
  ) as {
    x402Version: number;
    accepts: Array<{
      scheme: string;
      network: string;
      amount: string;
      asset: string;
      payTo: string;
      extra?: Record<string, unknown>;
    }>;
  };
  console.log("accepts:", paymentRequired.accepts.map((a) => a.network).join(", "));
  const challenge = paymentRequired.accepts.find(
    (a) => a.network === "solana-devnet",
  );
  if (!challenge) {
    throw new Error(
      "gateway did not advertise solana-devnet in 402 accepts; check X402_NETWORK env",
    );
  }
  console.log(
    `price: ${challenge.amount} atoms USDC -> ${challenge.payTo} on ${challenge.network}`,
  );

  logSection("Sign X-PAYMENT");
  let xPaymentHeader: string;
  if (createPaymentHeader) {
    console.log("(using payai x402-solana client)");
    xPaymentHeader = await createPaymentHeader({
      requirements: challenge,
      buyer,
      rpcUrl: RPC_URL,
    });
  } else {
    console.log("(payai x402-solana not installed — building header inline)");
    xPaymentHeader = await buildXPaymentInline(conn, buyer, challenge);
  }
  console.log("header length:", xPaymentHeader.length, "chars");

  logSection("Paid request");
  const ethTx =
    "0xf86c808504a817c800825208943535353535353535353535353535353535353535880de0b6b3a76400008025a028ef61340bd939bc2195fe537567866003e1a15d3c71ff63e1590620aa636276a067cbe9d8997f761aecb703304b3800ccf555c9f3dc64214b297fb1966a3b6d83";
  const paid = await fetch(`${GATEWAY_URL}/visualsign/api/v2/parse`, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      "X-PAYMENT": xPaymentHeader,
    },
    body: JSON.stringify({
      request: { unsigned_payload: ethTx, chain: "CHAIN_ETHEREUM" },
    }),
  });
  console.log("status:", paid.status);
  const xpr = paid.headers.get("X-PAYMENT-RESPONSE");
  if (xpr) {
    console.log("X-PAYMENT-RESPONSE:", xpr.slice(0, 80), xpr.length > 80 ? "…" : "");
  }
  const text = await paid.text();
  if (paid.status !== 200) {
    throw new Error(`paid request failed: ${paid.status} ${text}`);
  }
  const body = JSON.parse(text) as {
    response: {
      parsedTransaction: {
        signature: {
          publicKey: string;
          message: string;
          signature: string;
          scheme: string;
        };
        payload: { signablePayload: string };
      };
    };
  };
  const sig = body.response.parsedTransaction.signature;
  console.log("signature pubkey:", sig.publicKey.slice(0, 24), "…");

  if (TVC_HEX) {
    logSection("Independent P256 verification");
    if (sig.publicKey.toLowerCase() !== TVC_HEX) {
      throw new Error(
        `response pubkey ${sig.publicKey} != X402_TVC_VERIFIER_PUBKEY_HEX`,
      );
    }
    // qos_p256 encodes P256Public as encrypt_public || sign_public, each SEC1
    // uncompressed (65 bytes). The signature is over the message bytes; the
    // sign half is the second 65 bytes. @noble/curves verifies prehashed
    // messages via the .verify path; both signer and verifier hash again.
    const pubBytes = Buffer.from(sig.publicKey, "hex");
    if (pubBytes.length !== 130) {
      throw new Error(
        `expected 130-byte P256Public, got ${pubBytes.length}; aborting cross-check`,
      );
    }
    const signHalf = pubBytes.subarray(65, 130);
    const msgBytes = Buffer.from(sig.message, "hex");
    const sigBytes = Buffer.from(sig.signature, "hex");
    const ok = p256.verify(sigBytes, msgBytes, signHalf, { prehash: false });
    if (!ok) {
      throw new Error("independent P256 verification FAILED");
    }
    console.log("response signature verifies against pinned TVC pubkey ✓");
  }

  logSection("Done");
  console.log("payload bytes:", body.response.parsedTransaction.payload.signablePayload.length);
}

async function buildXPaymentInline(
  conn: Connection,
  buyer: Keypair,
  challenge: {
    scheme: string;
    network: string;
    amount: string;
    asset: string;
    payTo: string;
    extra?: Record<string, unknown>;
  },
): Promise<string> {
  // Fallback hand-built X-PAYMENT. Mirrors what payai's x402-solana would
  // produce: an SPL Token v1 transfer from buyer ATA to receiver ATA, signed
  // by the buyer only; the facilitator fills the fee-payer slot at /settle.
  const { TOKEN_PROGRAM_ID, createTransferInstruction, getAssociatedTokenAddress } =
    await import("@solana/spl-token");
  const { TransactionMessage } = await import("@solana/web3.js");

  const mint = new PublicKey(challenge.asset);
  const receiver = new PublicKey(challenge.payTo);
  const buyerAta = await getAssociatedTokenAddress(mint, buyer.publicKey);
  const receiverAta = await getAssociatedTokenAddress(mint, receiver);
  const amount = BigInt(challenge.amount);

  const ix = createTransferInstruction(
    buyerAta,
    receiverAta,
    buyer.publicKey,
    amount,
    [],
    TOKEN_PROGRAM_ID,
  );

  const blockhash = (await conn.getLatestBlockhash("confirmed")).blockhash;
  const message = new TransactionMessage({
    payerKey: buyer.publicKey,
    recentBlockhash: blockhash,
    instructions: [ix],
  }).compileToLegacyMessage();

  const tx = new VersionedTransaction(message);
  tx.sign([buyer]);
  const txB64 = Buffer.from(tx.serialize()).toString("base64");

  const payload = {
    x402Version: 2,
    scheme: challenge.scheme,
    network: challenge.network,
    payload: { transaction: txB64 },
    accepted: challenge,
  };
  return Buffer.from(JSON.stringify(payload)).toString("base64");
}

main().catch((e) => {
  console.error("FATAL:", e);
  process.exitCode = 1;
});
