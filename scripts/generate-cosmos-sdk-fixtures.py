#!/usr/bin/env python3
"""Generate schema-derived compatibility fixtures; never loads parser structs.

Historical schemas are fetched only to record their source hashes. Amino bytes
are canonicalized by Go encoding/json, matching SDK SortJSON's JSON roundtrip.
Protobuf uses the field numbers documented in the pinned upstream tx.proto.
These fixtures test parsing/display, not a running chain's ValidateBasic rules.
"""
import argparse
import concurrent.futures
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

SOURCES = {
    "bank030": ("v0.30.0", "x/bank/msgs.go"),
    "bank032": ("v0.32.0", "x/bank/msgs.go"),
    "staking030": ("v0.30.0", "x/staking/types/msg.go"),
    "staking032": ("v0.32.0", "x/staking/types/msg.go"),
    "staking033": ("v0.33.0", "x/staking/types/msg.go"),
    "staking034": ("v0.34.0", "x/staking/types/msg.go"),
    "vote039": ("v0.39.3", "x/gov/types/vote.go"),
    "tx040": ("v0.40.0", "proto/cosmos/tx/v1beta1/tx.proto"),
    "gov040": ("v0.40.0", "proto/cosmos/gov/v1beta1/tx.proto"),
    "tx046": ("v0.46.0", "proto/cosmos/tx/v1beta1/tx.proto"),
    "gov046": ("v0.46.0", "proto/cosmos/gov/v1/tx.proto"),
    "tx050": ("v0.50.0", "proto/cosmos/tx/v1beta1/tx.proto"),
    "tx053": ("v0.53.0", "proto/cosmos/tx/v1beta1/tx.proto"),
    "tx055": ("v0.55.0", "proto/cosmos/tx/v1beta1/tx.proto"),
}
COIN = {"amount": "1000000", "denom": "uatom"}
FROM = "cosmos190vqdjtlpcq27xslcveglfmr4ynfwg7gqmchsn"
TO = "cosmos1sxmr0k8u6trd5c6eu6trzyapzux7090y3u5dan"
VALIDATOR = "cosmosvaloper1historical"


def varint(n):
    out = bytearray()
    while n >= 128:
        out.append((n & 127) | 128)
        n >>= 7
    out.append(n)
    return bytes(out)


def b(tag, value):
    value = value.encode() if isinstance(value, str) else value
    return varint(tag * 8 + 2) + varint(len(value)) + value


def u(tag, value):
    return varint(tag * 8) + varint(value) if value else b""


def any_message(name, value):
    return b(1, name) + b(2, value)


def direct(message, extra_body=b"", tip=False):
    coin = b(1, "uatom") + b(2, "5000")
    key = any_message("/cosmos.crypto.secp256k1.PubKey", b(1, bytes([2]) * 33))
    signer = b(1, key) + b(2, b(1, u(1, 1))) + u(3, 7)
    auth = b(1, signer) + b(2, b(1, coin) + u(2, 200000))
    if tip:
        auth += b(3, b(1, coin) + b(2, FROM))
    body = b(1, message) + extra_body
    return (b(1, body) + b(2, auth) + b(3, "compat-test-1") + u(4, 42)).hex()


def amino(message):
    return {"account_number": "42", "chain_id": "compat-test-1", "sequence": "7",
            "fee": {"amount": [{"amount": "5000", "denom": "uatom"}], "gas": "200000"},
            "memo": "historical SDK fixture", "msgs": [message]}


def build_fixtures():
    rows = []
    def add(name, sdk, sources, kind, doc, fields, partial=False):
        rows.append({"id": name, "sdk": sdk, "source_ids": sources, "format": kind,
                     "sign_doc": doc, "fields": fields, "sigil_partial": partial})
    def am(name, sdk, sources, ty, value, fields):
        add(name, sdk, sources, "amino", amino({"type": ty, "value": value}), fields)
    am("sdk030-bank-inputs-outputs", "v0.30.0", ["bank030"], "cosmos-sdk/Send",
       {"inputs": [{"address": FROM, "coins": [COIN]}],
        "outputs": [{"address": TO, "coins": [COIN]}]},
       [["Message 1 / Input 1 / Address", FROM], ["Message 1 / Output 1 / Address", TO]])
    am("sdk032-bank-single-send", "v0.32.0", ["bank032"], "cosmos-sdk/Send",
       {"from_address": FROM, "to_address": TO, "amount": [COIN]}, [["Message 1 / To", TO]])
    am("sdk030-delegate-value", "v0.30.0", ["staking030"], "cosmos-sdk/MsgDelegate",
       {"delegator_addr": FROM, "validator_addr": VALIDATOR, "value": COIN}, [["Message 1 / Amount", "1000000 uatom"]])
    am("sdk032-undelegate-shares", "v0.32.0", ["staking032"], "cosmos-sdk/Undelegate",
       {"delegator_addr": FROM, "validator_addr": VALIDATOR, "shares_amount": "1.250000000000000000"},
       [["Message 1 / Delegation shares", "1.250000000000000000"]])
    am("sdk033-redelegate-shares", "v0.33.0", ["staking033"], "cosmos-sdk/MsgBeginRedelegate",
       {"delegator_address": FROM, "validator_src_address": VALIDATOR,
        "validator_dst_address": "cosmosvaloper1destination", "shares_amount": "9007199254740993.123456789012345678"},
       [["Message 1 / Delegation shares", "9007199254740993.123456789012345678"]])
    am("sdk034-undelegate-tokens", "v0.34.0", ["staking034"], "cosmos-sdk/MsgUndelegate",
       {"delegator_address": FROM, "validator_address": VALIDATOR, "amount": COIN}, [["Message 1 / Amount", "1000000 uatom"]])
    for value, expected in [("Yes", "Yes"), ("Abstain", "Abstain"), ("No", "No"), ("NoWithVeto", "No with veto")]:
        am("sdk039-vote-" + value.lower(), "v0.39.3", ["vote039"], "cosmos-sdk/MsgVote",
           {"proposal_id": "42", "voter": FROM, "option": value}, [["Message 1 / Option", expected]])
    send = any_message("/cosmos.bank.v1beta1.MsgSend", b(1, FROM) + b(2, TO) + b(3, b(1, "uatom") + b(2, "1000000")))
    for version, source in [("v0.40.0", "tx040"), ("v0.50.0", "tx050"), ("v0.55.0", "tx055")]:
        add("sdk" + version[1:].replace(".", "") + "-direct-send", version, [source], "direct", direct(send), [["Message 1 / To", TO]])
    vote = b(1, "/cosmos.gov.v1beta1.MsgVote") + b(2, u(1, 42) + b(2, FROM) + u(3, 1))
    add("sdk040-gov-v1beta1", "v0.40.0", ["gov040", "tx040"], "direct", direct(vote), [["Message 1 / Option", "Yes"]])
    modern_vote = any_message("/cosmos.gov.v1.MsgVote", u(1, 42) + b(2, FROM) + u(3, 1) + b(4, "ipfs://compatibility"))
    add("sdk046-gov-v1-metadata", "v0.46.0", ["gov046", "tx046"], "direct", direct(modern_vote), [["Message 1 / Metadata", "ipfs://compatibility"]])
    add("sdk046-tip", "v0.46.0", ["tx046"], "direct", direct(send, tip=True), [["Tip (deprecated)", "5000 uatom from " + FROM]], partial=True)
    timestamp = u(1, 1750000000) + u(2, 123)
    add("sdk053-unordered", "v0.53.0", ["tx053"], "direct", direct(send, u(4, 1) + b(5, timestamp)), [["Unordered", "yes"], ["Timeout timestamp", "1750000000s 123ns (unix)"]])
    return rows


def fetch_source(item):
    name, (tag, path) = item
    url = f"https://raw.githubusercontent.com/cosmos/cosmos-sdk/{tag}/{path}"
    data = subprocess.check_output(["curl", "--fail", "--silent", "--show-error", "--location", "--max-time", "30", url])
    return {"id": name, "tag": tag, "path": path, "url": url,
            "sha256": hashlib.sha256(data).hexdigest()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    rows = build_fixtures()
    docs = [row["sign_doc"] for row in rows if row["format"] == "amino"]
    go = '''package main
import("encoding/json";"os")
func main(){ var docs []any; if e:=json.NewDecoder(os.Stdin).Decode(&docs);e!=nil{panic(e)}; result:=make([]string,len(docs));for i,d:=range docs {b,e:=json.Marshal(d);if e!=nil{panic(e)};result[i]=string(b)};if e:=json.NewEncoder(os.Stdout).Encode(result);e!=nil{panic(e)} }
'''
    with tempfile.TemporaryDirectory(prefix="cosmos-fixture-go-") as folder:
        folder = Path(folder); (folder / "main.go").write_text(go)
        env = dict(os.environ, GOCACHE=str(folder / "cache"), GOTOOLCHAIN="local", GOENV="off")
        result = subprocess.run(["go", "run", str(folder / "main.go")], input=json.dumps(docs), text=True,
                                capture_output=True, env=env, check=True)
        canonical = iter(json.loads(result.stdout))
    for row in rows:
        if row["format"] == "amino": row["sign_doc"] = next(canonical)
        encoded = row["sign_doc"].encode() if row["format"] == "amino" else bytes.fromhex(row["sign_doc"])
        row["sha256"] = hashlib.sha256(encoded).hexdigest()
    with concurrent.futures.ThreadPoolExecutor(max_workers=6) as pool:
        sources = list(pool.map(fetch_source, SOURCES.items()))
    corpus = {"format_version": 1,
              "provenance": "Schema-derived synthetic fixtures, not SDK-produced or chain-executed transactions. Amino canonicalized independently using Go encoding/json; protobuf assembled independently from tagged field numbers. No parser structs used.",
              "sources": sources, "fixtures": rows}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(corpus, indent=2, ensure_ascii=True) + "\n")
    print(f"Wrote {len(rows)} fixtures with {len(sources)} tagged source hashes to {args.output}")

if __name__ == "__main__": main()
