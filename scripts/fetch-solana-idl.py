#!/usr/bin/env python3
"""Fetch the Anchor IDL a Solana program publishes on chain.

Usage:
    scripts/fetch-solana-idl.py <program_id> [--rpc URL] [--out FILE] [--compare FILE] [--provenance FILE]

    scripts/fetch-solana-idl.py --record-repo <https-url> --commit <40-hex-sha> \
        --path <path-in-repo> --idl <idl.json> --program-id <id>
    scripts/fetch-solana-idl.py --record-issuer '<who supplied it>' [--notes '<text>'] \
        --idl <idl.json> --program-id <id>
    scripts/fetch-solana-idl.py --record-unknown '<why the origin is unknown>' \
        --idl <idl.json> --program-id <id>

Reads the program's Anchor IDL account over JSON-RPC and prints the IDL JSON
(or writes it to --out). With --compare, also summarizes how it differs from a
committed IDL: instructions added, removed or changed. Exits 2 when the program
has no on-chain IDL.

Provenance records
------------------

Every IDL compiled into a Solana preset must carry a provenance record next to
it: `<file>.json` is accompanied by `<file>.provenance.json`, which says where
the IDL came from. A Rust test (builtin_programs.rs) refuses an IDL whose
record is missing, stale, or disagrees with the IDL's sha256, so the IDL and
its record can only change together. This script writes all four kinds:

- "onchain" -- written together with the IDL it fetched. Needs --out; the
  record hashes the exact bytes written and captures the RPC slot plus the
  slot the program was last deployed at.
- "repo" -- the IDL was copied from a public repository at a known commit.
- "issuer" -- the IDL was supplied out of band by a named party.
- "unknown" -- the origin can't be established; the record keeps that gap
  visible instead of hiding it.

The record modes take no network: they hash an existing --idl file and write
the record next to it (as `<idl>.provenance.json`).

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
import os
import re
import sys
import urllib.request
import zlib
from datetime import datetime, timezone

DEFAULT_RPC = "https://api.mainnet-beta.solana.com"
IDL_SEED = b"anchor:idl"
PDA_MARKER = b"ProgramDerivedAddress"
HEADER_LEN = 8 + 32 + 4

# Owner program whose accounts hold upgradeable Solana programs, and the tags
# its accounts carry: 2 = "Program" (36 bytes: tag + ProgramData address),
# 3 = "ProgramData" (tag, u64 LE last-deploy slot, Option<Pubkey> authority).
BPF_UPGRADEABLE_LOADER = "BPFLoaderUpgradeab1e11111111111111111111111"
PROGRAM_TAG = 2
PROGRAM_DATA_TAG = 3
PROGRAM_DATA_SLICE = {"offset": 0, "length": 45}

PROVENANCE_TOOL = "scripts/fetch-solana-idl.py"
FULL_SHA = re.compile(r"[0-9a-f]{40}")

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


def rpc_get_account_info(rpc: str, address: str, data_slice=None):
    """One getAccountInfo call, as (data bytes, owner, context.slot).

    data is None when the account does not exist. data_slice limits the
    returned bytes (base64 encoding still applies).
    """
    account = {"encoding": "base64", "commitment": "finalized"}
    if data_slice is not None:
        account["dataSlice"] = data_slice
    body = json.dumps({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getAccountInfo",
        "params": [address, account],
    }).encode()
    req = urllib.request.Request(rpc, data=body, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=60) as resp:
        reply = json.load(resp)
    if "error" in reply:
        raise RuntimeError(f"RPC error: {reply['error']}")
    slot = reply["result"]["context"]["slot"]
    value = reply["result"]["value"]
    if value is None:
        return None, None, slot
    return base64.b64decode(value["data"][0]), value["owner"], slot


def rpc_account_data(rpc: str, address: str):
    data, _, _ = rpc_get_account_info(rpc, address)
    return data


def program_data_address(owner: str, data: bytes):
    """ProgramData account address an upgradeable program points at, or None.

    A program owned by the upgradeable BPF loader holds a little-endian u32
    tag 2 ("Program") followed by the 32-byte ProgramData address. Any other
    owner or tag means the program is not upgradeable.
    """
    if owner != BPF_UPGRADEABLE_LOADER:
        return None
    if len(data) < 4 + 32:
        return None
    if int.from_bytes(data[:4], "little") != PROGRAM_TAG:
        return None
    return data[4:36]


def deployed_slot_from_program_data(data: bytes):
    """Last-deployment slot from ProgramData account bytes, or None.

    ProgramData holds a little-endian u32 tag 3 ("ProgramData"), a u64 LE slot
    (the last deployment slot), then an Option<Pubkey> upgrade authority. The
    RPC call slices the first 45 bytes so the program binary that follows is
    never fetched. Any other tag (or truncated data) means no slot.
    """
    if len(data) < 4 + 8:
        return None
    if int.from_bytes(data[:4], "little") != PROGRAM_DATA_TAG:
        return None
    return int.from_bytes(data[4:12], "little")


def program_last_deployed_slot(rpc: str, program_id: str):
    """Slot the program was last deployed at, or None.

    None whenever the program is not upgradeable or the two RPC lookups fail
    for any reason -- the slot is context in a provenance record, never a
    reason to fail a fetch.
    """
    try:
        data, owner, _ = rpc_get_account_info(rpc, program_id)
        if data is None:
            return None
        address = program_data_address(owner, data)
        if address is None:
            return None
        program_data, _, _ = rpc_get_account_info(
            rpc, b58encode(address), data_slice=PROGRAM_DATA_SLICE
        )
        if program_data is None:
            return None
        return deployed_slot_from_program_data(program_data)
    except Exception:
        return None


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


def utc_now_iso() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def sha256_of_file(path: str) -> str:
    with open(path, "rb") as fh:
        return hashlib.sha256(fh.read()).hexdigest()


def record_path_for(idl_path: str) -> str:
    """Sibling provenance path: `<file>.json` -> `<file>.provenance.json`."""
    stem, ext = os.path.splitext(idl_path)
    if ext != ".json":
        raise ValueError(f"--idl must be a .json file: {idl_path}")
    return stem + ".provenance.json"


def new_record(program_id: str, idl_sha256: str, source: str) -> dict:
    """Common provenance fields, in the canonical record key order."""
    return {
        "program_id": program_id,
        "idl_sha256": idl_sha256,
        "source": source,
        "recorded_at": utc_now_iso(),
    }


def write_record(path: str, record: dict) -> None:
    """Write a provenance record: JSON, 2-space indent, trailing newline."""
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(json.dumps(record, indent=2) + "\n")


def run_fetch(args) -> int:
    address = idl_address(args.program_id)
    data, _, rpc_slot = rpc_get_account_info(args.rpc, address)
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
    if args.provenance:
        if not args.out:
            print(
                "error: --provenance requires --out (the record hashes the exact IDL bytes written)",
                file=sys.stderr,
            )
            return 1
        last_deployed = program_last_deployed_slot(args.rpc, args.program_id)
        record = new_record(args.program_id, hashlib.sha256(text.encode("utf-8")).hexdigest(), "onchain")
        record["idl_account"] = address
        record["rpc_slot"] = rpc_slot
        record["program_last_deployed_slot"] = last_deployed
        record["tool"] = PROVENANCE_TOOL
        write_record(args.provenance, record)
        print(f"provenance record: {args.provenance}", file=sys.stderr)
        print(f"rpc slot: {rpc_slot}", file=sys.stderr)
        slot_text = last_deployed if last_deployed is not None else "unknown"
        print(f"program last deployed slot: {slot_text}", file=sys.stderr)
    return 0


def run_record_mode(args, mode_count: int) -> int:
    def fail(message: str) -> int:
        print(f"error: {message}", file=sys.stderr)
        return 1

    if mode_count > 1:
        return fail("pass at most one of --record-repo, --record-issuer, --record-unknown")
    if args.program_id:
        return fail("record modes take the program id via --program-id, not the positional argument")
    if (args.commit or args.path) and args.record_repo is None:
        return fail("--commit and --path only apply to --record-repo")
    if args.notes and args.record_issuer is None:
        return fail("--notes only applies to --record-issuer")
    for flag, value in [("--out", args.out), ("--compare", args.compare), ("--provenance", args.provenance)]:
        if value:
            return fail(f"{flag} only applies to an on-chain fetch, not to a record mode")
    if not args.idl or not args.record_program_id:
        return fail("record modes need --idl FILE and --program-id ID")
    if not os.path.isfile(args.idl):
        return fail(f"{args.idl} not found")

    try:
        program = b58decode(args.record_program_id)
    except ValueError as e:
        return fail(f"invalid --program-id: {e}")
    if len(program) != 32:
        return fail(f"--program-id must decode to 32 bytes, got {len(program)}")

    record = new_record(args.record_program_id, sha256_of_file(args.idl), "unknown")
    if args.record_repo is not None:
        if not args.record_repo.startswith("https://"):
            return fail("--record-repo must be an https URL")
        if args.commit is None or args.path is None:
            return fail("--record-repo needs --commit and --path")
        if not FULL_SHA.fullmatch(args.commit):
            return fail("--commit must be a full 40-character lowercase hex SHA")
        if not args.path:
            return fail("--path must not be empty")
        record["source"] = "repo"
        record["repo"] = args.record_repo
        record["commit"] = args.commit
        record["path"] = args.path
    elif args.record_issuer is not None:
        if not args.record_issuer:
            return fail("--record-issuer must not be empty")
        record["source"] = "issuer"
        record["supplied_by"] = args.record_issuer
        record["notes"] = args.notes
    else:
        if not args.record_unknown:
            return fail("--record-unknown must not be empty (say why the origin is unknown)")
        record["source"] = "unknown"
        record["notes"] = args.record_unknown

    path = record_path_for(args.idl)
    write_record(path, record)
    print(f"provenance record: {path}", file=sys.stderr)
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("program_id", nargs="?", help="program id to fetch (base58)")
    parser.add_argument("--rpc", default=DEFAULT_RPC, help=f"JSON-RPC endpoint (default {DEFAULT_RPC})")
    parser.add_argument("--out", help="write the IDL JSON here instead of stdout")
    parser.add_argument("--compare", help="committed IDL JSON to summarize differences against (to stderr)")
    parser.add_argument(
        "--provenance",
        help="with --out, also write an 'onchain' provenance record here for the IDL just written",
    )
    parser.add_argument(
        "--record-repo",
        help="write a 'repo' provenance record for an existing IDL: https URL of the repo it came from",
    )
    parser.add_argument("--commit", help="full 40-hex commit of the repo the IDL came from (with --record-repo)")
    parser.add_argument("--path", help="path of the IDL inside that repo (with --record-repo)")
    parser.add_argument(
        "--record-issuer",
        help="write an 'issuer' provenance record for an existing IDL: free text naming who supplied it",
    )
    parser.add_argument(
        "--record-unknown",
        help="write an 'unknown' provenance record for an existing IDL: free text on why the origin can't be established",
    )
    parser.add_argument("--notes", default="", help="free-text notes for --record-issuer")
    parser.add_argument("--idl", help="existing IDL file the record mode describes")
    parser.add_argument(
        "--program-id",
        dest="record_program_id",
        help="program id (base58) the record mode writes the record for",
    )
    args = parser.parse_args()

    record_modes = [
        args.record_repo is not None,
        args.record_issuer is not None,
        args.record_unknown is not None,
    ]
    if any(record_modes):
        return run_record_mode(args, sum(record_modes))
    if args.program_id is None:
        parser.error("program_id is required unless recording a provenance record")
    return run_fetch(args)


if __name__ == "__main__":
    sys.exit(main())
