#!/usr/bin/env python3
"""Package real mainnet Zcash transactions as PCZT replay fixtures.

Reads files you fetch yourself from the Blockchair API (this script opens no
network connection) and writes mainnet_txs.json next to this file:

    ./gen_mainnet_fixtures.py --print-fetch     # curl lines for the pinned txids
    ./gen_mainnet_fixtures.py <dir>             # <dir>/raw/<txid>.json, <dir>/dash/<txid>.json

For each transaction the fixture keeps its raw hex and the explorer's own view
of it: each transparent input's spent output (value, script, address), each
transparent output, the per-pool value balances the node decoded, and the
explorer's fee for transparent-only transactions. The Rust test rebuilds a PCZT
from the raw transaction and the spent outputs, recomputes the txid, and checks
the parser (and the ZIP-244 digests it shows) against those numbers and against
the transaction's own on-chain signatures.
"""

import json
import sys
from pathlib import Path

API = "https://api.blockchair.com/zcash"
# Mainnet transactions, each a different shape. v5 ones are from just before
# NU6.3 (block 3,428,143) or from October 2026; v6 ones are post-NU6.3.
TXIDS = {
    "v5-transparent-1in-2out": "965b835bebc49290bf60b139b6240dfd99c7263bbfd28167b1cfb726f7dc93ef",
    "v5-transparent-5in-2out": "850da586e5378880ab779bfda96e0fb1b674ee96165f7cbf129042c14e272056",
    "v6-transparent-1in-2out": "4ba50aab7744718257ba4744e302b4fbc8df501b16231be48389ed00abd81c06",
    "v5-sapling-z2z": "f19451dbb1808228463a298e79d5c7947957ad4ce9ef409a6095e93a6eb82bc0",
    "v5-sapling-t2z": "e3f35b0843c09785ca6bca3d507848c87f1fe51e3b69ec4cc132cb6e3a7464d4",
    "v5-sapling-z2t": "0cb8a4a93cd1ca56c0ed9ced2dfe5aa565f53b525e46d2ed58d20a5be7a11a00",
    "v5-orchard-z2z": "42b15d5e843e73cd42f353933a5bd3447352ba934ae6aeecaab5c3f692034aa8",
    "v5-orchard-t2z": "eb0edaa32f400073aef5220cfefdccd4a6b62a41f7a3e7c82d7670ac872920e7",
    "v5-orchard-z2t": "31d318827a86902560d8033a87249945745b686338dc1492a5700d959690de62",
    "v6-ironwood-z2z": "bf33aa5a87c6a854245c2c562acd43310be6e4f598bdcaf17da6e85739cfe542",
}


def pool(decoded, key):
    p = decoded.get(key) or {}
    return {"actions": len(p.get("actions", [])), "value_balance": p.get("valueBalanceZat", 0)}


def main():
    if sys.argv[1:] == ["--print-fetch"]:
        for txid in TXIDS.values():
            print(f"curl -sSf {API}/raw/transaction/{txid} -o raw/{txid}.json")
            print(f"curl -sSf {API}/dashboards/transaction/{txid} -o dash/{txid}.json")
        return 0
    if len(sys.argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    src = Path(sys.argv[1])
    out = []
    for label, txid in TXIDS.items():
        raw = json.loads((src / "raw" / f"{txid}.json").read_text())["data"][txid]
        dash = json.loads((src / "dash" / f"{txid}.json").read_text())["data"][txid]
        dec = raw["decoded_raw_transaction"]
        spent = {(i["transaction_hash"], i["index"]): i for i in dash["inputs"]}
        inputs = []
        for vin in dec["vin"]:
            prev = spent[(vin["txid"], vin["vout"])]
            inputs.append(
                {
                    "txid": vin["txid"],
                    "vout": vin["vout"],
                    "address": prev["recipient"],
                    "type": prev["type"],
                    "script_hex": prev["script_hex"],
                    "value_zat": prev["value"],
                }
            )
        transparent_only = not (
            dec["vShieldedSpend"]
            or dec["vShieldedOutput"]
            or pool(dec, "orchard")["actions"]
            or pool(dec, "ironwood")["actions"]
        )
        out.append(
            {
                "label": label,
                "txid": txid,
                "block_height": dash["transaction"]["block_id"],
                "tx_hex": raw["raw_transaction"],
                "explorer": {
                    "version": dec["version"],
                    # Blockchair reports 0 for shielded transactions' fees.
                    "fee_zat": dash["transaction"]["fee"] if transparent_only else None,
                    "inputs": inputs,
                    "outputs": [
                        {
                            "address": (o["scriptPubKey"].get("addresses") or [None])[0],
                            "type": o["scriptPubKey"]["type"],
                            "script_hex": o["scriptPubKey"]["hex"],
                            "value_zat": o["valueZat"],
                        }
                        for o in dec["vout"]
                    ],
                    "sapling": {
                        "spends": len(dec["vShieldedSpend"]),
                        "outputs": len(dec["vShieldedOutput"]),
                        "value_balance": dec["valueBalanceZat"],
                    },
                    "orchard": pool(dec, "orchard"),
                    "ironwood": pool(dec, "ironwood"),
                },
            }
        )
    doc = {"source": f"{API} (fetched 2026-10-23)", "txs": out}
    dest = Path(__file__).with_name("mainnet_txs.json")
    dest.write_text(json.dumps(doc, indent=2) + "\n")
    print(f"wrote {len(out)} transactions to {dest}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
