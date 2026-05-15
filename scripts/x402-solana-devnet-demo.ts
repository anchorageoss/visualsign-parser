#!/usr/bin/env -S npx tsx
/**
 * x402 demo client against the visualsign-parser gateway with payai
 * facilitator on Solana devnet.
 *
 * Drives the same flow as the gated Rust integration test
 * (`tests/x402_payai_devnet_test.rs`) but outside cargo so humans can poke
 * at it without rebuilding the workspace.
 *
 * Uses payai's `x402-solana` v2.x client (the supported reference TS impl).
 * `createX402Client(...).fetch(url)` handles the 402 challenge, payment
 * construction, settlement retry, and response in a single call.
 *
 * Prerequisites:
 *   1. The gateway is running locally with X402_PROFILE=payai and
 *      X402_NETWORK=solana-devnet. Easiest: `make dev-up-payai` from the
 *      repo root.
 *   2. The reproducible test wallet from `src/integration/fixtures/devnet/`
 *      is funded with devnet SOL (faucet.solana.com) and devnet USDC
 *      (faucet.circle.com, mint 4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU).
 *      Current address: x2iWww6XjauBk83HpBMzkGPijbzy4vqdRzS5skWPxmW
 *
 * Env vars (all optional):
 *   GATEWAY_URL   default http://127.0.0.1:8080
 *   RPC_URL       default https://api.devnet.solana.com
 *   WALLET_SEED   default ../src/integration/fixtures/devnet/wallet.seed
 *   X402_TVC_VERIFIER_PUBKEY_HEX
 *                 if set, the demo independently P256-verifies the response
 *                 signature against this key (cross-impl check vs the
 *                 gateway).
 */

import { Connection, Keypair, VersionedTransaction } from "@solana/web3.js";
import { fileURLToPath } from "node:url";
import { readFile } from "node:fs/promises";
import { p256 } from "@noble/curves/p256";
import { sha256 } from "@noble/hashes/sha2";
import { dirname, resolve } from "node:path";
import { createX402Client, type WalletAdapter } from "x402-solana/client";

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

function buildWalletAdapter(buyer: Keypair): WalletAdapter {
  return {
    publicKey: buyer.publicKey,
    signTransaction: async (tx: VersionedTransaction) => {
      tx.sign([buyer]);
      return tx;
    },
  };
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
      `WARNING: low SOL balance (${lamports} lamports). \`solana airdrop 2 ${buyer.publicKey.toBase58()} --url devnet\` or https://faucet.solana.com`,
    );
  }

  logSection("x402 client (payai/x402-solana)");
  const client = createX402Client({
    wallet: buildWalletAdapter(buyer),
    network: "solana-devnet",
    rpcUrl: RPC_URL,
    verbose: true,
  });
  console.log("client constructed; making paid request…");

  logSection("Paid POST /visualsign/api/v2/parse");
  const ethTx =
    "0xf86c808504a817c800825208943535353535353535353535353535353535353535880de0b6b3a76400008025a028ef61340bd939bc2195fe537567866003e1a15d3c71ff63e1590620aa636276a067cbe9d8997f761aecb703304b3800ccf555c9f3dc64214b297fb1966a3b6d83";
  const resp = await client.fetch(`${GATEWAY_URL}/visualsign/api/v2/parse`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      request: { unsigned_payload: ethTx, chain: "CHAIN_ETHEREUM" },
    }),
  });

  console.log("status:", resp.status);
  // x402-axum v2 emits the settlement summary on `Payment-Response`.
  // Older clients spell it `X-PAYMENT-RESPONSE` — check both.
  const settlementHeader =
    resp.headers.get("Payment-Response") ?? resp.headers.get("X-PAYMENT-RESPONSE");
  if (settlementHeader) {
    console.log("Payment-Response (b64):", settlementHeader.slice(0, 120) + (settlementHeader.length > 120 ? "…" : ""));
    try {
      const decoded = JSON.parse(
        Buffer.from(settlementHeader, "base64").toString("utf-8"),
      );
      console.log("settlement:", JSON.stringify(decoded, null, 2));
    } catch (e) {
      console.warn("could not decode Payment-Response as JSON:", e);
    }
  } else {
    console.warn("no Payment-Response header on the 200 — facilitator may not have echoed it");
  }

  const text = await resp.text();
  if (resp.status !== 200) {
    throw new Error(`paid request failed: ${resp.status} ${text}`);
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

  if (TVC_HEX) {
    logSection("Independent P256 verification");
    if (sig.publicKey.toLowerCase() !== TVC_HEX) {
      throw new Error(
        `response pubkey ${sig.publicKey} != X402_TVC_VERIFIER_PUBKEY_HEX`,
      );
    }
    // qos_p256 encodes P256Public as encrypt_public || sign_public, each
    // SEC1 uncompressed (65 bytes = 130 hex chars). The sign half is the
    // second 65 bytes.
    //
    // Hashing: parser_app builds `digest = sha256(borsh(payload))` and calls
    // `P256Pair::sign(&digest)`. P256SignPair::sign forwards to
    // `p256::ecdsa::SigningKey::sign(msg)`, whose default `Signer<Signature>`
    // impl applies SHA-256 to `msg` again before signing. So the signed
    // value is actually `sha256(digest)`. To verify with @noble/curves,
    // hash one more time on this side and pass the prehash explicitly.
    const pubBytes = Buffer.from(sig.publicKey, "hex");
    if (pubBytes.length !== 130) {
      throw new Error(
        `expected 130-byte P256Public, got ${pubBytes.length}; aborting cross-check`,
      );
    }
    const signHalf = pubBytes.subarray(65, 130);
    const digest = Buffer.from(sig.message, "hex");
    const inner = sha256(digest);
    const sigBytes = Buffer.from(sig.signature, "hex");
    const ok = p256.verify(sigBytes, inner, signHalf);
    if (!ok) {
      throw new Error("independent P256 verification FAILED");
    }
    console.log("response signature verifies against pinned TVC pubkey ✓");
  }

  logSection("Done");
  console.log(
    "payload bytes:",
    body.response.parsedTransaction.payload.signablePayload.length,
  );
}

main().catch((e) => {
  console.error("FATAL:", e);
  process.exitCode = 1;
});
