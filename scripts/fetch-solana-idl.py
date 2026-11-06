#!/usr/bin/env python3
"""Fetch the Anchor IDL a Solana program publishes on chain.

Usage:
    scripts/fetch-solana-idl.py <program_id> [--rpc URL] [--out FILE] [--compare FILE]

Reads the program's Anchor IDL account over JSON-RPC and prints the IDL JSON
(or writes it to --out). With --compare, also summarizes how it differs from a
committed IDL: instructions added, removed or changed. Exits 2 when the program
has no on-chain IDL.

Standard library only, so it runs anywhere without installing the Anchor CLI.
The IDL account is found the way Anchor finds it:

    base    = find_program_address([], program_id)
    address = create_with_seed(base, "anchor:idl", program_id)

Its data is an 8-byte discriminator, the 32-byte authority, a little-endian
u32 length, then that many bytes of zlib-compressed IDL JSON.
"""

import argparse
import base64
import hashlib
import json
import sys
import urllib.request
import zlib

DEFAULT_RPC = "https://api.mainnet-beta.solana.com"
IDL_SEED = b"anchor:idl"
PDA_MARKER = b"ProgramDerivedAddress"
HEADER_LEN = 8 + 32 + 4

B58_ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"

# ed25519 field prime and curve constant, for the on-curve check that
# find_program_address uses to reject candidate addresses.
P = 2**255 - 19
D = (-121665 * pow(121666, P - 2, P)) % P


def b58decode(text: str) -> bytes:
    num = 0
    for ch in text:
        idx = B58_ALPHABET.find(ch)
        if idx < 0:
            raise ValueError(f"invalid base58 character {ch!r}")
        num = num * 58 + idx
    raw = num.to_bytes((num.bit_length() + 7) // 8, "big") if num else b""
    pad = len(text) - len(text.lstrip("1"))
    return b"\x00" * pad + raw


def b58encode(data: bytes) -> str:
    num = int.from_bytes(data, "big")
    out = ""
    while num:
        num, rem = divmod(num, 58)
        out = B58_ALPHABET[rem] + out
    pad = len(data) - len(data.lstrip(b"\x00"))
    return "1" * pad + out


def is_on_curve(point: bytes) -> bool:
    """True if `point` decompresses to a point on the ed25519 curve."""
    y = int.from_bytes(point, "little") & ((1 << 255) - 1)
    if y >= P:
        return False
    y2 = y * y % P
    u = (y2 - 1) % P
    v = (D * y2 + 1) % P
    # x^2 = u / v; a square root exists iff (u/v)^((p-1)/2) is 0 or 1.
    x2 = u * pow(v, P - 2, P) % P
    if x2 == 0:
        return True
    return pow(x2, (P - 1) // 2, P) == 1


def find_program_address(seeds: list, program_id: bytes) -> bytes:
    for bump in range(255, -1, -1):
        digest = hashlib.sha256(b"".join(seeds) + bytes([bump]) + program_id + PDA_MARKER).digest()
        if not is_on_curve(digest):
            return digest
    raise RuntimeError("no viable bump seed")


def create_with_seed(base: bytes, seed: bytes, owner: bytes) -> bytes:
    return hashlib.sha256(base + seed + owner).digest()


def idl_address(program_id: str) -> str:
    program = b58decode(program_id)
    if len(program) != 32:
        raise ValueError(f"program id must decode to 32 bytes, got {len(program)}")
    base = find_program_address([], program)
    return b58encode(create_with_seed(base, IDL_SEED, program))


def rpc_account_data(rpc: str, address: str):
    body = json.dumps({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getAccountInfo",
        "params": [address, {"encoding": "base64", "commitment": "finalized"}],
    }).encode()
    req = urllib.request.Request(rpc, data=body, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=60) as resp:
        reply = json.load(resp)
    if "error" in reply:
        raise RuntimeError(f"RPC error: {reply['error']}")
    value = reply["result"]["value"]
    if value is None:
        return None
    return base64.b64decode(value["data"][0])


def decode_idl_account(data: bytes) -> dict:
    if len(data) < HEADER_LEN:
        raise ValueError(f"IDL account is {len(data)} bytes, shorter than its {HEADER_LEN}-byte header")
    length = int.from_bytes(data[40:44], "little")
    compressed = data[HEADER_LEN:HEADER_LEN + length]
    if len(compressed) != length:
        raise ValueError(f"IDL account declares {length} bytes but holds {len(compressed)}")
    return json.loads(zlib.decompress(compressed))


def instruction_signatures(idl: dict) -> dict:
    sigs = {}
    for ix in idl.get("instructions", []):
        sigs[ix["name"]] = (
            tuple(ix.get("discriminator") or []),
            tuple(a.get("name") for a in ix.get("args", [])),
            tuple(a.get("name") for a in ix.get("accounts", [])),
        )
    return sigs


def compare(onchain: dict, committed: dict) -> str:
    new, old = instruction_signatures(onchain), instruction_signatures(committed)
    added = sorted(set(new) - set(old))
    removed = sorted(set(old) - set(new))
    changed = sorted(k for k in set(new) & set(old) if new[k] != old[k])
    same = json.dumps(onchain, sort_keys=True) == json.dumps(committed, sort_keys=True)
    lines = [
        f"identical: {'yes' if same else 'no'}",
        f"instructions: committed {len(old)}, on chain {len(new)}",
        f"added: {', '.join(added) or '-'}",
        f"removed: {', '.join(removed) or '-'}",
        f"changed: {', '.join(changed) or '-'}",
    ]
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("program_id")
    parser.add_argument("--rpc", default=DEFAULT_RPC, help=f"JSON-RPC endpoint (default {DEFAULT_RPC})")
    parser.add_argument("--out", help="write the IDL JSON here instead of stdout")
    parser.add_argument("--compare", help="committed IDL JSON to summarize differences against (to stderr)")
    args = parser.parse_args()

    address = idl_address(args.program_id)
    data = rpc_account_data(args.rpc, address)
    if data is None:
        print(f"{args.program_id} has no on-chain Anchor IDL (account {address} not found)", file=sys.stderr)
        return 2
    idl = decode_idl_account(data)
    text = json.dumps(idl, indent=2, ensure_ascii=False) + "\n"
    if args.out:
        with open(args.out, "w", encoding="utf-8") as fh:
            fh.write(text)
    else:
        sys.stdout.write(text)
    print(f"IDL account: {address}", file=sys.stderr)
    if args.compare:
        with open(args.compare, encoding="utf-8") as fh:
            print(compare(idl, json.load(fh)), file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
