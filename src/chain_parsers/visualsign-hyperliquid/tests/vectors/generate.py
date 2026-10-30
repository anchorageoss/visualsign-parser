"""Generate golden vectors for visualsign-hyperliquid from the official Python SDK.

Provenance: hyperliquid-python-sdk==0.24.0 (PyPI), msgpack 1.2.4, CPython 3.11.

The SDK's own Exchange methods build every action they support. Network calls are
stubbed out: `Exchange.post` captures the request body instead of sending it, and
`signing.sign_inner` is wrapped so the EIP-712 typed data the SDK signs is recorded
verbatim. Clock and randomness are fixed so the output is reproducible.

Actions the 0.24.0 SDK has no builder for (single `modify`, `twapOrder`,
`twapCancel`, `cancel` with `f`, HIP-4 `outcomeDeploy` and `activateOutcomeDeployer`)
are written by hand from the official exchange-endpoint and HIP-4 docs, then signed
through the SDK's `sign_l1_action`, so their connectionId still comes from the SDK's
`action_hash`.

Usage (in a venv with the SDK installed):

    python -I generate.py > vectors.json
"""

import json
import sys

import eth_account

import hyperliquid.exchange as hl_exchange
import hyperliquid.utils.signing as signing
from hyperliquid.exchange import Exchange
from hyperliquid.utils.constants import MAINNET_API_URL, TESTNET_API_URL
from hyperliquid.utils.types import Cloid

SDK_VERSION = "0.24.0"

# Throwaway key, never used for anything real.
WALLET = eth_account.Account.from_key("0x" + "11" * 32)
AGENT_KEY_HEX = "22" * 32
NONCE = 1761480000000  # 2025-10-26T12:00:00Z
EXPIRES_AFTER = 1761483600000  # one hour later
VAULT = "0x1719884eb866cb12b2287399b15f7db5e7d775ea"
SUB_ACCOUNT = "0x4b2c5a9a8f0e1d3c6b7a8f9e0d1c2b3a4f5e6d7c"
DESTINATION = "0x5e9ee1089755c3435139848e47e6635505d5a13a"
VALIDATOR = "0x5ac99df645f3414876c816caa18b2d234024b487"
BUILDER = "0x8c967e73e7b15087c42a10d344cff4c96d877f1d"

META = {"universe": [{"name": "BTC", "szDecimals": 5}, {"name": "ETH", "szDecimals": 4}]}
SPOT_META = {
    "universe": [{"name": "PURR/USDC", "tokens": [1, 0], "index": 0}],
    "tokens": [
        {"name": "USDC", "szDecimals": 8, "weiDecimals": 8, "index": 0},
        {"name": "PURR", "szDecimals": 0, "weiDecimals": 5, "index": 1},
    ],
}

captured_posts = []
captured_typed = []

_orig_sign_inner = signing.sign_inner


def _recording_sign_inner(wallet, data):
    captured_typed.append(json.loads(json.dumps(data, default=_bytes_to_hex)))
    return _orig_sign_inner(wallet, data)


def _bytes_to_hex(value):
    if isinstance(value, (bytes, bytearray)):
        return "0x" + bytes(value).hex()
    raise TypeError(f"unserializable {type(value)}")


signing.sign_inner = _recording_sign_inner
hl_exchange.get_timestamp_ms = lambda: NONCE
hl_exchange.secrets.token_hex = lambda n: AGENT_KEY_HEX[: 2 * n]
Exchange.post = lambda self, path, payload: captured_posts.append(payload) or {"status": "ok"}


def exchange(mainnet=True, vault=None, expires_after=None):
    ex = Exchange(
        WALLET,
        MAINNET_API_URL if mainnet else TESTNET_API_URL,
        meta=META,
        vault_address=vault,
        spot_meta=SPOT_META,
    )
    ex.set_expires_after(expires_after)
    return ex


vectors = []


def record(name, kind):
    """Pair the last captured request body with the last captured typed data."""
    post = captured_posts.pop()
    typed = captured_typed.pop()
    assert not captured_posts and not captured_typed, name
    if kind == "l1":
        payload = {
            "kind": "l1",
            "typedData": typed,
            "action": post["action"],
            "nonce": post["nonce"],
            "vaultAddress": post["vaultAddress"],
            "expiresAfter": post["expiresAfter"],
        }
        # The SDK sends vaultAddress for every L1 action, but vaultTransfer and
        # subAccountTransfer are signed without it. Use what was signed.
        if post["action"]["type"] in ("vaultTransfer", "subAccountTransfer") or post["action"]["type"].endswith("Deploy"):
            payload["vaultAddress"] = None
    else:
        payload = {"kind": "userSigned", "typedData": typed}
    vectors.append(
        {
            "name": name,
            "unsignedPayload": json.dumps(payload, separators=(",", ":")),
            "signature": post["signature"],
            "signer": WALLET.address,
        }
    )


def hand_l1(name, action, mainnet=True, vault=None, expires_after=None):
    """Sign a hand-built action through the SDK's sign_l1_action."""
    signature = signing.sign_l1_action(WALLET, action, vault, NONCE, expires_after, mainnet)
    captured_posts.append(
        {
            "action": action,
            "nonce": NONCE,
            "signature": signature,
            "vaultAddress": vault,
            "expiresAfter": expires_after,
        }
    )
    record(name, "l1")


# ---- L1 actions built by the SDK ----
exchange().order("BTC", True, 0.001, 100000, {"limit": {"tif": "Gtc"}})
record("l1_order_limit_mainnet", "l1")

exchange(mainnet=False).order(
    "ETH",
    False,
    1.25,
    4000.5,
    {"trigger": {"triggerPx": 3900, "isMarket": True, "tpsl": "sl"}},
    reduce_only=True,
    cloid=Cloid.from_int(7),
    builder={"b": BUILDER, "f": 10},
)
record("l1_order_trigger_cloid_builder_testnet", "l1")

exchange().bulk_orders(
    [
        {"coin": "PURR/USDC", "is_buy": True, "sz": 100, "limit_px": 0.2, "order_type": {"limit": {"tif": "Alo"}}, "reduce_only": False},
        {"coin": "BTC", "is_buy": False, "sz": 0.5, "limit_px": 120000, "order_type": {"limit": {"tif": "Ioc"}}, "reduce_only": False},
    ],
    grouping="normalTpsl",
)
record("l1_order_bulk_spot_and_perp", "l1")

exchange(vault=VAULT).order("BTC", True, 0.01, 99000, {"limit": {"tif": "Gtc"}})
record("l1_order_vault", "l1")

exchange(expires_after=EXPIRES_AFTER).order("BTC", True, 0.01, 99000, {"limit": {"tif": "Gtc"}})
record("l1_order_expires_after", "l1")

exchange(vault=VAULT, expires_after=EXPIRES_AFTER).cancel("BTC", 123456789)
record("l1_cancel_vault_expires_after", "l1")

exchange().cancel("ETH", 42)
record("l1_cancel", "l1")

exchange().cancel_by_cloid("BTC", Cloid.from_int(255))
record("l1_cancel_by_cloid", "l1")

exchange().bulk_modify_orders_new(
    [
        {"oid": 77, "order": {"coin": "BTC", "is_buy": True, "sz": 0.002, "limit_px": 98000, "order_type": {"limit": {"tif": "Gtc"}}, "reduce_only": False, "cloid": None}},
        {"oid": Cloid.from_int(9), "order": {"coin": "ETH", "is_buy": False, "sz": 3, "limit_px": 4100, "order_type": {"limit": {"tif": "Alo"}}, "reduce_only": False, "cloid": Cloid.from_int(9)}},
    ]
)
record("l1_batch_modify", "l1")

exchange().schedule_cancel(NONCE + 60_000)
record("l1_schedule_cancel_time", "l1")

exchange().schedule_cancel(None)
record("l1_schedule_cancel_unset", "l1")

exchange().update_leverage(10, "ETH", is_cross=False)
record("l1_update_leverage", "l1")

exchange().update_isolated_margin(-12.5, "BTC")
record("l1_update_isolated_margin", "l1")

exchange().vault_usd_transfer(VAULT, True, 5_000_000)
record("l1_vault_transfer", "l1")

exchange().sub_account_transfer(SUB_ACCOUNT, False, 1_000_000)
record("l1_sub_account_transfer", "l1")

exchange().noop(NONCE)
record("l1_noop", "l1")

# spotDeploy (HIP-1/HIP-2)
exchange().spot_deploy_register_token("TEST", 2, 8, 1_000_000_000_000, "Test Token")
record("l1_spot_deploy_register_token2", "l1")
exchange().spot_deploy_user_genesis(1234, [(SUB_ACCOUNT, "1000000"), (DESTINATION, "250")], [(0, "5000")])
record("l1_spot_deploy_user_genesis", "l1")
exchange().spot_deploy_genesis(1234, "1000250", True)
record("l1_spot_deploy_genesis", "l1")
exchange().spot_deploy_register_spot(1234, 0)
record("l1_spot_deploy_register_spot", "l1")
exchange().spot_deploy_register_hyperliquidity(321, 2.5, 100.0, 50, 5)
record("l1_spot_deploy_register_hyperliquidity", "l1")
exchange().spot_deploy_set_deployer_trading_fee_share(1234, "50%")
record("l1_spot_deploy_set_fee_share", "l1")
exchange().spot_deploy_enable_freeze_privilege(1234)
record("l1_spot_deploy_enable_freeze_privilege", "l1")
exchange().spot_deploy_freeze_user(1234, DESTINATION, True)
record("l1_spot_deploy_freeze_user", "l1")
exchange().spot_deploy_revoke_freeze_privilege(1234)
record("l1_spot_deploy_revoke_freeze_privilege", "l1")
exchange().spot_deploy_enable_quote_token(1234)
record("l1_spot_deploy_enable_quote_token", "l1")

# perpDeploy (HIP-3)
exchange().perp_deploy_register_asset(
    "abc", None, "abc:XYZ", 3, "12.5", 1, False, {"fullName": "ABC dex", "collateralToken": 0, "oracleUpdater": None}
)
record("l1_perp_deploy_register_asset_new_dex", "l1")
exchange().perp_deploy_register_asset("abc", 1_000_000, "abc:QRS", 2, "3.25", 2, True, None)
record("l1_perp_deploy_register_asset_existing_dex", "l1")
exchange().perp_deploy_set_oracle(
    "abc",
    {"abc:XYZ": "12.6", "abc:QRS": "3.3"},
    [{"abc:XYZ": "12.55", "abc:QRS": "3.29"}],
    {"abc:XYZ": "12.61", "abc:QRS": "3.31"},
)
record("l1_perp_deploy_set_oracle", "l1")

# ---- L1 actions the SDK has no builder for (shapes from the official docs) ----
hand_l1(
    "l1_modify_single",
    {
        "type": "modify",
        "oid": 77,
        "order": {"a": 0, "b": True, "p": "98500", "s": "0.002", "r": False, "t": {"limit": {"tif": "Gtc"}}},
    },
)
hand_l1("l1_cancel_fast", {"type": "cancel", "cancels": [{"a": 1, "o": 5}], "f": True})
hand_l1("l1_twap_order", {"type": "twapOrder", "twap": {"a": 0, "b": True, "s": "0.5", "r": False, "m": 30, "t": True}})
hand_l1("l1_twap_cancel", {"type": "twapCancel", "a": 0, "t": 1234})
hand_l1("l1_activate_outcome_deployer", {"type": "activateOutcomeDeployer", "activate": {"venueName": "ab"}}, mainnet=False)
hand_l1("l1_deactivate_outcome_deployer", {"type": "activateOutcomeDeployer", "deactivate": None}, mainnet=False)
hand_l1(
    "l1_outcome_deploy_register_standalone",
    {
        "type": "outcomeDeploy",
        "venue": "ab",
        "operation": {
            "registerStandaloneOutcomeFromTemplate": {
                "id": "abc",
                "keywordToValue": [["expiry", "20260801-0600"], ["target", "100"], ["underlying", "ABC"]],
                "deployerFeeScale": "1",
            }
        },
    },
    mainnet=False,
)
hand_l1(
    "l1_outcome_deploy_register_question",
    {
        "type": "outcomeDeploy",
        "venue": "ab",
        "operation": {
            "registerQuestionFromTemplate": {
                "questionTemplateInstance": {"id": "abc", "keywordToValue": [["expiry", "20260801-1830"]], "deployerFeeScale": "1"},
                "namedOutcomeTemplateInstances": [
                    {"id": "abc-outcome", "keywordToValue": [["choice", "A"]]},
                    {"id": "abc-outcome", "keywordToValue": [["choice", "B"]]},
                    {"id": "abc-other", "keywordToValue": []},
                ],
            }
        },
    },
    mainnet=False,
)
hand_l1(
    "l1_outcome_deploy_settle_outcome",
    {
        "type": "outcomeDeploy",
        "venue": "ab",
        "operation": {
            "settleOutcome": {
                "outcome": 7,
                "settleFraction": "1",
                "details": "",
                "nameAndDescription": ["template:abc", "expiry:20260801-0600|target:100|underlying:ABC"],
                "sideNames": ["template:Over", "template:Under"],
            }
        },
    },
    mainnet=False,
)
hand_l1(
    "l1_outcome_deploy_set_sub_deployers",
    {
        "type": "outcomeDeploy",
        "venue": "ab",
        "operation": {
            "setSubDeployers": [
                {"variant": "settleOutcome", "user": SUB_ACCOUNT, "allowed": True},
            ]
        },
    },
    mainnet=False,
)

# ---- Validly hashed actions the parser must refuse (names start with "refuse_") ----
# A real SDK action type that is not on the allowlist.
exchange().set_referrer("ABC")
record("refuse_l1_set_referrer_not_allowlisted", "l1")
# An allowlisted type with a field the display structs do not know.
hand_l1(
    "refuse_l1_order_unknown_field",
    {
        "type": "order",
        "orders": [{"a": 0, "b": True, "p": "1", "s": "1", "r": False, "t": {"limit": {"tif": "Gtc"}}}],
        "grouping": "na",
        "extra": 1,
    },
)
# `time: null` hashes differently from an omitted key (what the SDK sends to unset);
# the display struct cannot tell them apart, so the round-trip check must refuse it.
hand_l1("refuse_l1_schedule_cancel_null_time", {"type": "scheduleCancel", "time": None})
# Key order differs from the SDK's; the hash is valid for these bytes, but the typed
# re-encode writes wire order, so the round-trip check must refuse it.
hand_l1("refuse_l1_twap_cancel_reordered_keys", {"type": "twapCancel", "t": 1234, "a": 0})

# ---- User-signed actions built by the SDK ----
exchange().usd_transfer(1.5, DESTINATION)
record("user_usd_send", "userSigned")
exchange(mainnet=False).spot_transfer(10.0, DESTINATION, "PURR:0xc4bf3f870c0e9465323c0b6ed28096c2")
record("user_spot_send_testnet", "userSigned")
exchange().withdraw_from_bridge(100.0, DESTINATION)
record("user_withdraw", "userSigned")
exchange().usd_class_transfer(25.0, True)
record("user_usd_class_transfer", "userSigned")
exchange().send_asset(DESTINATION, "", "spot", "USDC", 7.0)
record("user_send_asset", "userSigned")
exchange().token_delegate(VALIDATOR, 100_000_000, False)
record("user_token_delegate", "userSigned")
exchange().approve_agent("bot valid_until 1767225600000")
record("user_approve_agent_valid_until", "userSigned")
exchange().approve_agent(None)
record("user_approve_agent_no_name", "userSigned")
exchange().approve_builder_fee(BUILDER, "0.001%")
record("user_approve_builder_fee", "userSigned")

json.dump(
    {
        "provenance": {
            "generator": "tests/vectors/generate.py",
            "hyperliquidPythonSdk": SDK_VERSION,
            "msgpack": __import__("msgpack").version,
        },
        "vectors": vectors,
    },
    sys.stdout,
    indent=1,
)
sys.stdout.write("\n")
