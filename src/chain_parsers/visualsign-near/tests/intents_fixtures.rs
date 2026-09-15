//! Every NEP-413 envelope under `tests/fixtures/` renders.
//!
//! Half are recorded from the live NEAR Intents app at near.com and half are
//! constructed from `defuse_core::intents`, one per intent kind
//! `visualsign_intents::render` handles. `tests/fixtures/README.md` says which
//! is which and how the recordings were made.
//!
//! An intents envelope is the whole of what a self-custody signer approves, so
//! the assertions are the facts they need to decide: which intent, to whom, and
//! how much of what. A kind that stops rendering, or renders without naming its
//! counterparty or amount, fails here.
//!
//! Lives in `tests/` rather than the crate's test module because it is about the
//! fixture directory as a set: the final case walks the directory and fails on a
//! file no case claims, so a fixture added without a test cannot go unnoticed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use visualsign::vsptrait::{VisualSignConverterFromString, VisualSignOptions};
use visualsign_near::NearVisualSignConverter;

/// What a fixture must show a signer, beyond rendering at all.
struct Expected {
    file: &'static str,
    title: &'static str,
    /// `(label, substring)` pairs the rendered payload has to carry.
    shows: &'static [(&'static str, &'static str)],
}

const FIXTURES: &[Expected] = &[
    // -- Recorded from near.com ------------------------------------------------
    Expected {
        // Sign-in: an empty batch, which executes nothing but burns the nonce.
        // It still has to render, because it is the first thing a user signs.
        file: "near-com-signin-verification.json",
        title: "NEAR Intent",
        shows: &[("NEP-413 Recipient", "intents.near")],
    },
    Expected {
        // Moving a balance between accounts inside intents.near. The `tokens`
        // map is keyed by defuse asset id, which is the shape the app produces
        // and which hand-written fixtures had not covered.
        file: "near-com-transfer-intent.json",
        title: "NEAR Intent: Transfer",
        shows: &[
            ("Intent", "Transfer"),
            (
                "To",
                "bc285a4d4e98bd8ea3fb4590917628a98400e8c3c85a88ab4b1af783603113ad",
            ),
            // nep141:wrap.near at 24 decimals, resolved rather than shown raw.
            ("Amount", "25 wNEAR"),
            ("NEP-413 Recipient", "intents.near"),
        ],
    },
    Expected {
        // A legacy-token migration: an ft_withdraw whose receiver is
        // intents.near itself, so the balance lands back on the same account
        // under the current token. The memo is prose the app composes.
        file: "near-com-ft-withdraw-migration.json",
        title: "NEAR Intent: FT Withdraw",
        shows: &[
            ("Intent", "FT Withdraw"),
            ("Token", "aurora"),
            ("To", "intents.near"),
            ("Memo", "Migrate ETH"),
            (
                "Message",
                "524a505623102a2026b3fd2ddede369c71531214d1cde397598070c4b1099f48",
            ),
        ],
    },
    // -- Constructed, one per intent kind -------------------------------------
    Expected {
        // A swap. Negative deltas are spent and positive received, and the two
        // sides carry different decimals, so a renderer that resolved only one
        // of them would still pass a single-token case.
        file: "intent-token-diff.json",
        title: "NEAR Intent: Token Diff",
        shows: &[
            ("Send", "1 wNEAR"),
            ("Receive", "2.4 USDC"),
            ("Referral", "near-intents.near"),
        ],
    },
    Expected {
        file: "intent-transfer-multi-token.json",
        title: "NEAR Intent: Transfer",
        shows: &[
            ("To", "bob.near"),
            ("Amount", "1 wNEAR"),
            ("Amount", "2.4 USDC"),
            ("Memo", "rent"),
        ],
    },
    Expected {
        file: "intent-ft-withdraw-with-storage-deposit.json",
        title: "NEAR Intent: FT Withdraw",
        shows: &[
            ("Token", "wrap.near"),
            ("To", "bob.near"),
            ("Amount", "1 wNEAR"),
            // A separate, unconditional wNEAR debit, never refunded on failure.
            ("Storage Deposit", "0.00125 NEAR"),
        ],
    },
    Expected {
        file: "intent-nft-withdraw.json",
        title: "NEAR Intent: NFT Withdraw",
        shows: &[
            ("Token", "nft.near"),
            ("To", "bob.near"),
            ("NFT Token Id", "1337"),
        ],
    },
    Expected {
        // token_ids and amounts are parallel vectors, so each pairing has to
        // reach the signer unambiguously or an amount could be read against the
        // wrong token. An amount this build cannot resolve names the asset id it
        // belongs to, which is what carries the pairing.
        file: "intent-mt-withdraw.json",
        title: "NEAR Intent: MT Withdraw",
        shows: &[
            ("Token", "mt.near"),
            ("To", "bob.near"),
            ("MT Token", "series-1"),
            ("Amount", "3 (unresolved nep245:mt.near:series-1)"),
            ("MT Token", "series-2"),
            ("Amount", "5 (unresolved nep245:mt.near:series-2)"),
        ],
    },
    Expected {
        file: "intent-native-withdraw.json",
        title: "NEAR Intent: Native Withdraw",
        shows: &[("To", "bob.near"), ("Amount", "1 NEAR")],
    },
    Expected {
        file: "intent-storage-deposit.json",
        title: "NEAR Intent: Storage Deposit",
        shows: &[("Contract", "wrap.near"), ("For Account", "bob.near")],
    },
    Expected {
        file: "intent-add-public-key.json",
        title: "NEAR Intent: Add Public Key",
        shows: &[(
            "Add Public Key",
            "ed25519:8rVvtHWFr8hasdQGGD5WiQBTyr4iH2ruEPPVfj491RPN",
        )],
    },
    Expected {
        file: "intent-remove-public-key.json",
        title: "NEAR Intent: Remove Public Key",
        shows: &[(
            "Remove Public Key",
            "ed25519:8rVvtHWFr8hasdQGGD5WiQBTyr4iH2ruEPPVfj491RPN",
        )],
    },
    Expected {
        file: "intent-set-auth-by-predecessor-id.json",
        title: "NEAR Intent: Set Auth By Predecessor Id",
        shows: &[("Auth By Predecessor", "disabled")],
    },
    Expected {
        file: "intent-auth-call.json",
        title: "NEAR Intent: Auth Call",
        shows: &[("Contract", "vault.near"), ("Message", "claim")],
    },
    Expected {
        // Two intents under one signature: the signer approves the batch or
        // none of it, so both have to render.
        file: "intent-batch-swap-then-withdraw.json",
        title: "NEAR Intent",
        shows: &[("Send", "1 wNEAR"), ("Receive", "2.4 USDC"), ("To", "bob.near")],
    },
    Expected {
        file: "intent-empty-batch.json",
        title: "NEAR Intent",
        shows: &[("NEP-413 Recipient", "intents.near")],
    },
];

fn fixture_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

#[test]
fn every_fixture_renders_what_a_signer_needs_to_decide() {
    for case in FIXTURES {
        let raw = std::fs::read_to_string(fixture_dir().join(case.file))
            .unwrap_or_else(|e| panic!("{}: {e}", case.file));

        let payload = NearVisualSignConverter::new()
            .to_visual_sign_payload_from_string(raw.trim(), VisualSignOptions::default())
            .unwrap_or_else(|e| panic!("{} must render: {e:?}", case.file))
            .payload;

        assert_eq!(payload.title, case.title, "{}", case.file);

        for (label, want) in case.shows {
            let found = payload
                .fields
                .iter()
                .any(|f| f.label() == *label && f.fallback_text().contains(want));
            assert!(
                found,
                "{}: no field labelled {label:?} containing {want:?}; got {:?}",
                case.file, payload.fields
            );
        }
    }
}

/// The four intents that change who controls an account carry a warning, not
/// only their own fields. Asserted apart from the rendering cases because it is
/// a property of the kind rather than of any one fixture.
#[test]
fn the_account_control_intents_warn() {
    const ACCOUNT_CONTROL: &[&str] = &[
        "intent-add-public-key.json",
        "intent-remove-public-key.json",
        "intent-set-auth-by-predecessor-id.json",
        "intent-auth-call.json",
    ];

    for file in ACCOUNT_CONTROL {
        let raw = std::fs::read_to_string(fixture_dir().join(file)).expect("fixture");
        let payload = NearVisualSignConverter::new()
            .to_visual_sign_payload_from_string(raw.trim(), VisualSignOptions::default())
            .unwrap_or_else(|e| panic!("{file} must render: {e:?}"))
            .payload;

        let json = payload.to_json().expect("json");
        assert!(
            json.contains("account-control"),
            "{file} must carry an account-control warning: {json}"
        );
    }
}

/// A fixture nothing claims is a fixture nothing tests. Walking the directory
/// rather than trusting the list above is what makes the set self-maintaining.
#[test]
fn no_fixture_is_left_untested() {
    let mut on_disk: Vec<String> = std::fs::read_dir(fixture_dir())
        .expect("fixture dir")
        .map(|entry| entry.expect("entry").file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".json"))
        .collect();
    on_disk.sort();

    let mut claimed: Vec<String> = FIXTURES.iter().map(|c| c.file.to_string()).collect();
    claimed.sort();

    assert_eq!(
        on_disk, claimed,
        "every .json under tests/fixtures must appear in FIXTURES"
    );
}
