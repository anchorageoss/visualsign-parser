#!/usr/bin/env python3
"""Extract the PSBT test vectors from BIP-174, BIP-370 and BIP-371.

Reads the BIP mediawiki files from a directory you fetch yourself (this script
opens no network connection), checks their sha256 against the pins below, and
writes bip_vectors.json next to this file.

    ./gen_bip_vectors.py --print-fetch          # curl lines for the pinned commit
    ./gen_bip_vectors.py <dir-with-mediawiki>   # regenerate bip_vectors.json

Each vector is a case from a "Test Vectors" section, tagged "invalid" or "valid"
by the section it appears in. In the valid sections only blobs that start with
the PSBT magic are kept (the BIP-174 workflow section also shows keys and the
extracted network transaction). Hex and base64 renderings of the same case are
deduplicated by their decoded bytes.
"""

import base64
import binascii
import hashlib
import json
import re
import sys
from pathlib import Path

REPO = "bitcoin/bips"
COMMIT = "c0c5aea41e6a334d34db4ca87ae53bbfd7f268d1"
FILES = {
    "bip-0174.mediawiki": "f2a8e1a9c9e31cc7f607b3c7e2419c63eeccc4bd5b725968cde54ab8cfa1d410",
    "bip-0370.mediawiki": "b2b6e9099592fa04179383f9a1fd8b201fbeef4470f4c3f1c050d25589979c57",
    "bip-0371.mediawiki": "c2e8143a3c9aee0ee38d863cf40947ddf8ee67b4f40a748fb6925ccc5ad4b791",
}
MAGIC = b"psbt\xff"
PRE = re.compile(r"<pre>\s*([0-9A-Za-z+/=\s]+?)\s*</pre>")


def decode(blob):
    blob = "".join(blob.split())
    if re.fullmatch(r"(?:[0-9a-fA-F]{2})+", blob):
        return binascii.unhexlify(blob)
    try:
        return base64.b64decode(blob, validate=True)
    except binascii.Error:
        return None


def extract(name, text):
    lines = text.splitlines()
    start = next(i for i, l in enumerate(lines) if re.match(r"^==+ *Test Vectors *==+", l))
    status, case, seen, out = None, None, set(), []
    for line in lines[start + 1 :]:
        if re.match(r"^==[^=]", line):
            break
        low = line.lower()
        if "following are invalid" in low:
            status = "invalid"
        elif "following are valid" in low:
            status = "valid"
        m = re.match(r"^\*+\s*Case:\s*(.*)$", line)
        if m:
            case = m.group(1).strip()
        elif line and not line.startswith("*") and not line.startswith("<pre>") and "<pre>" not in line:
            case = line.strip().rstrip(":")
        for blob in PRE.findall(line):
            raw = decode(blob)
            if raw is None or status is None:
                continue
            if status == "valid" and not raw.startswith(MAGIC):
                continue
            key = (status, raw)
            if key in seen:
                continue
            seen.add(key)
            out.append(
                {
                    "bip": name.split(".")[0],
                    "status": status,
                    "case": case,
                    "psbt_base64": base64.b64encode(raw).decode(),
                }
            )
    return out


def main():
    if sys.argv[1:] == ["--print-fetch"]:
        for f in FILES:
            print(f"curl -sSfLO https://raw.githubusercontent.com/{REPO}/{COMMIT}/{f}")
        return 0
    if len(sys.argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    src = Path(sys.argv[1])
    vectors = []
    for f, want in FILES.items():
        data = (src / f).read_bytes()
        got = hashlib.sha256(data).hexdigest()
        if got != want:
            print(f"{f}: sha256 {got} != pinned {want}", file=sys.stderr)
            return 1
        vectors.extend(extract(f, data.decode()))
    doc = {"source": {"repo": REPO, "commit": COMMIT, "sha256": FILES}, "vectors": vectors}
    out = Path(__file__).with_name("bip_vectors.json")
    out.write_text(json.dumps(doc, indent=2) + "\n")
    print(f"wrote {len(vectors)} vectors to {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
