#!/usr/bin/env python3
"""Package real mainnet transactions as PSBT test fixtures.

Reads files you fetch yourself from the mempool.space API (this script opens no
network connection) and writes mainnet_txs.json next to this file:

    ./gen_mainnet_fixtures.py --print-fetch     # curl lines for the pinned txids
    ./gen_mainnet_fixtures.py <dir>             # <dir>/api/<txid>.json, <dir>/raw/<txid>.hex

For each transaction the fixture keeps its raw hex, the raw hex of every
previous transaction it spends, and the explorer's own view of it (fee, each
output's address/type/value, each input's prevout address/value). The Rust test
rebuilds an unsigned PSBT from the raw transactions and checks the parser's
rendering against the explorer's numbers, an independent source. The test also
recomputes every txid from the raw hex, so a corrupted fixture fails there.
"""

import json
import sys
from pathlib import Path

API = "https://mempool.space/api"
# One small transaction per script type, all from mainnet block 972688
# (00000000000000000000bae3c51a7c2d83944f4d0f57196a6030ed3bb8596d32).
TXIDS = {
    "p2wpkh": "d3934a5a417626e997d9fa7aba12be78760b7e2046a1d2eac8c2a2acb1d3ead7",
    "p2pkh": "fb23252d7dcc43eae4e302a0596b8308f71e2ebb3b9928272091c43770a9e3f5",
    "p2wsh": "190609a270f2712374c4aa0e4593b2069dd26451513e86ff5f8f844337bf0498",
    "p2sh-p2wpkh": "8c2a84ca9f07f75dca32c02e9e000d117c58bcdff15ac8645ec67cb1fa9f2ab4",
    "op_return": "7499bdd87aec84bf5798e7a6b042104d929dc89b346e20fc18f688d843698c11",
    "p2tr": "cf6ff71c2a9b75217f1d6a675077029482606883dbf0cb97ef9b6511f2a33d33",
}


def main():
    if sys.argv[1:] == ["--print-fetch"]:
        for txid in TXIDS.values():
            print(f"curl -sSf {API}/tx/{txid} -o api/{txid}.json")
            print(f"curl -sSf {API}/tx/{txid}/hex -o raw/{txid}.hex")
        print("# then, for every vin[].txid in api/*.json:")
        print(f"#   curl -sSf {API}/tx/<prev txid>/hex -o raw/<prev txid>.hex")
        return 0
    if len(sys.argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    src = Path(sys.argv[1])
    out = []
    for label, txid in TXIDS.items():
        api = json.loads((src / "api" / f"{txid}.json").read_text())
        prev_ids = sorted({v["txid"] for v in api["vin"]})
        out.append(
            {
                "label": label,
                "txid": txid,
                "block_hash": api["status"]["block_hash"],
                "tx_hex": (src / "raw" / f"{txid}.hex").read_text().strip(),
                "prev_txs": {p: (src / "raw" / f"{p}.hex").read_text().strip() for p in prev_ids},
                "explorer": {
                    "fee_sat": api["fee"],
                    "inputs": [
                        {
                            "txid": v["txid"],
                            "vout": v["vout"],
                            "address": v["prevout"].get("scriptpubkey_address"),
                            "type": v["prevout"]["scriptpubkey_type"],
                            "value_sat": v["prevout"]["value"],
                        }
                        for v in api["vin"]
                    ],
                    "outputs": [
                        {
                            "address": o.get("scriptpubkey_address"),
                            "type": o["scriptpubkey_type"],
                            "script_hex": o["scriptpubkey"],
                            "value_sat": o["value"],
                        }
                        for o in api["vout"]
                    ],
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
