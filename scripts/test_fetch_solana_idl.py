#!/usr/bin/env python3
"""Tests for the provenance paths of scripts/fetch-solana-idl.py.

Everything here runs without network access: the record-writing modes
(repo / issuer / unknown), their validation, and the ProgramData parsers
fed fixed bytes. The on-chain fetch itself is exercised by the
"Solana: fetch on-chain IDL" workflow instead.

Run with:

    python3 -I scripts/test_fetch_solana_idl.py
"""

import hashlib
import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "fetch-solana-idl.py"

# The System program id: valid base58, decodes to 32 zero bytes.
PROGRAM_ID = "11111111111111111111111111111111"

IDL_JSON = '{"address": "%s", "instructions": [{"name": "noop"}]}\n' % PROGRAM_ID


def load_module():
    """Import the script as a module (it lives outside any package, and
    `python3 -I` strips the usual import paths)."""
    spec = importlib.util.spec_from_file_location("fetch_solana_idl", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


MOD = load_module()


def upgradeable_program_account(address: bytes) -> bytes:
    """Program account data as the upgradeable BPF loader lays it out."""
    return MOD.PROGRAM_TAG.to_bytes(4, "little") + address


def program_data_account(slot: int, has_authority: bool = True) -> bytes:
    """ProgramData account data: tag 3, u64 LE slot, Option<Pubkey>."""
    authority = (b"\x01" + bytes(range(32))) if has_authority else b"\x00"
    return MOD.PROGRAM_DATA_TAG.to_bytes(4, "little") + slot.to_bytes(8, "little") + authority


class ProgramDataParsing(unittest.TestCase):
    def test_upgradeable_program_yields_program_data_address(self):
        address = bytes(range(32))
        data = upgradeable_program_account(address)
        self.assertEqual(
            MOD.program_data_address(MOD.BPF_UPGRADEABLE_LOADER, data), address
        )

    def test_other_owner_is_not_upgradeable(self):
        data = upgradeable_program_account(bytes(32))
        self.assertIsNone(
            MOD.program_data_address("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA", data)
        )

    def test_non_program_tag_is_rejected(self):
        data = MOD.PROGRAM_DATA_TAG.to_bytes(4, "little") + bytes(32)
        self.assertIsNone(MOD.program_data_address(MOD.BPF_UPGRADEABLE_LOADER, data))

    def test_short_program_account_is_rejected(self):
        self.assertIsNone(
            MOD.program_data_address(MOD.BPF_UPGRADEABLE_LOADER, bytes(8))
        )

    def test_program_data_yields_last_deployed_slot(self):
        slot = 293_000_123
        self.assertEqual(
            MOD.deployed_slot_from_program_data(program_data_account(slot)), slot
        )

    def test_program_data_without_authority_still_yields_slot(self):
        # The authority Option is after the slot; slicing 45 bytes may also
        # drop it, so a zero-authority or truncated tail must not matter.
        self.assertEqual(
            MOD.deployed_slot_from_program_data(program_data_account(7, has_authority=False)), 7
        )

    def test_program_data_wrong_tag_is_rejected(self):
        data = MOD.PROGRAM_TAG.to_bytes(4, "little") + (99).to_bytes(8, "little")
        self.assertIsNone(MOD.deployed_slot_from_program_data(data))

    def test_program_data_short_is_rejected(self):
        self.assertIsNone(MOD.deployed_slot_from_program_data(bytes(4)))


class RecordPath(unittest.TestCase):
    def test_record_sits_next_to_the_idl(self):
        self.assertEqual(
            MOD.record_path_for("presets/jupiter_swap/jupiter_agg_v6.json"),
            "presets/jupiter_swap/jupiter_agg_v6.provenance.json",
        )

    def test_non_json_idl_is_rejected(self):
        with self.assertRaises(ValueError):
            MOD.record_path_for("idl.txt")


class RecordModeBase(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.idl = Path(self.tmp.name) / "some_program.json"
        self.idl.write_text(IDL_JSON, encoding="utf-8")

    @property
    def record(self) -> Path:
        return self.idl.with_name("some_program.provenance.json")

    def run_script(self, *argv):
        return subprocess.run(
            [sys.executable, "-I", str(SCRIPT), *argv],
            capture_output=True,
            text=True,
        )

    def assert_common_fields(self, record, source):
        self.assertEqual(record["source"], source)
        self.assertEqual(record["program_id"], PROGRAM_ID)
        self.assertEqual(
            record["idl_sha256"], hashlib.sha256(self.idl.read_bytes()).hexdigest()
        )
        self.assertRegex(
            record["recorded_at"], r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$"
        )
        # Canonical form: 2-space indent, trailing newline, key order intact.
        raw = self.record.read_text(encoding="utf-8")
        self.assertEqual(raw, json.dumps(record, indent=2) + "\n")
        self.assertEqual(
            list(record.keys()),
            ["program_id", "idl_sha256", "source", "recorded_at"]
            + {
                "onchain": ["idl_account", "rpc_slot", "program_last_deployed_slot", "tool"],
                "repo": ["repo", "commit", "path"],
                "issuer": ["supplied_by", "notes"],
                "unknown": ["notes"],
            }[source],
        )


class RepoRecords(RecordModeBase):
    def test_record_repo_writes_a_valid_record(self):
        result = self.run_script(
            "--record-repo", "https://github.com/example/example",
            "--commit", "0" * 40,
            "--path", "idls/some_program.json",
            "--idl", str(self.idl),
            "--program-id", PROGRAM_ID,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.record.is_file(), result.stderr)
        record = json.loads(self.record.read_text(encoding="utf-8"))
        self.assert_common_fields(record, "repo")
        self.assertEqual(record["repo"], "https://github.com/example/example")
        self.assertEqual(record["commit"], "0" * 40)
        self.assertEqual(record["path"], "idls/some_program.json")

    def test_short_commit_is_rejected(self):
        result = self.run_script(
            "--record-repo", "https://github.com/example/example",
            "--commit", "0" * 39,
            "--path", "idls/some_program.json",
            "--idl", str(self.idl),
            "--program-id", PROGRAM_ID,
        )
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("--commit", result.stderr)
        self.assertFalse(self.record.exists())

    def test_uppercase_commit_is_rejected(self):
        result = self.run_script(
            "--record-repo", "https://github.com/example/example",
            "--commit", "0" * 39 + "A",
            "--path", "idls/some_program.json",
            "--idl", str(self.idl),
            "--program-id", PROGRAM_ID,
        )
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("--commit", result.stderr)

    def test_http_url_is_rejected(self):
        result = self.run_script(
            "--record-repo", "http://github.com/example/example",
            "--commit", "0" * 40,
            "--path", "idls/some_program.json",
            "--idl", str(self.idl),
            "--program-id", PROGRAM_ID,
        )
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("https", result.stderr)

    def test_missing_commit_is_rejected(self):
        result = self.run_script(
            "--record-repo", "https://github.com/example/example",
            "--path", "idls/some_program.json",
            "--idl", str(self.idl),
            "--program-id", PROGRAM_ID,
        )
        self.assertEqual(result.returncode, 1, result.stdout)


class IssuerAndUnknownRecords(RecordModeBase):
    def test_record_issuer_writes_a_valid_record(self):
        result = self.run_script(
            "--record-issuer", "Example Labs support",
            "--notes", "attached to an email thread",
            "--idl", str(self.idl),
            "--program-id", PROGRAM_ID,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        record = json.loads(self.record.read_text(encoding="utf-8"))
        self.assert_common_fields(record, "issuer")
        self.assertEqual(record["supplied_by"], "Example Labs support")
        self.assertEqual(record["notes"], "attached to an email thread")

    def test_record_unknown_writes_a_valid_record(self):
        result = self.run_script(
            "--record-unknown", "origin predates the provenance policy",
            "--idl", str(self.idl),
            "--program-id", PROGRAM_ID,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        record = json.loads(self.record.read_text(encoding="utf-8"))
        self.assert_common_fields(record, "unknown")
        self.assertEqual(record["notes"], "origin predates the provenance policy")

    def test_empty_unknown_notes_are_rejected(self):
        result = self.run_script(
            "--record-unknown", "",
            "--idl", str(self.idl),
            "--program-id", PROGRAM_ID,
        )
        self.assertEqual(result.returncode, 1, result.stdout)

    def test_multiple_record_modes_are_rejected(self):
        result = self.run_script(
            "--record-unknown", "x",
            "--record-issuer", "y",
            "--idl", str(self.idl),
            "--program-id", PROGRAM_ID,
        )
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertFalse(self.record.exists())

    def test_missing_idl_is_rejected(self):
        missing = str(Path(self.tmp.name) / "nope.json")
        result = self.run_script(
            "--record-unknown", "x",
            "--idl", missing,
            "--program-id", PROGRAM_ID,
        )
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("not found", result.stderr)

    def test_bad_program_id_is_rejected(self):
        result = self.run_script(
            "--record-unknown", "x",
            "--idl", str(self.idl),
            "--program-id", "not-base58!",
        )
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertFalse(self.record.exists())


if __name__ == "__main__":
    unittest.main()
