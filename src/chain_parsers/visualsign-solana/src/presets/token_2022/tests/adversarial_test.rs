// Adversarial tests for the Token-2022 instructions decoded since #574:
// Transfer, TransferChecked, TransferCheckedWithFee, Approve, ApproveChecked,
// Revoke, MintTo and Burn.
//
// The preview runs in a signer, so anything it shows that the signed
// instruction does not say is a finding, and so is any panic. Most cases are
// proptest properties (256 cases by default, `PROPTEST_CASES=N` to override,
// as elsewhere in visualsign-solana). Two oracles are used:
//
// - `interface_decode`: `TokenInstruction::unpack` / `TransferFeeInstruction::unpack`
//   from spl-token-2022 10 (spl-token-2022-interface 2.1). The preset is built
//   on these, so they say what it *should* decode and with which fields. They
//   are lenient (they ignore trailing bytes), so they are not ground truth:
//   `should_decode` adds the program's exact-length rule and its fee <= amount
//   rule on top.
// - `program_decodes`: the Token-2022 program's own entrypoint,
//   `spl_token_2022::processor::Processor::process`, called with no accounts.
//   It decodes the data first and only then asks for accounts, so
//   `NotEnoughAccountKeys` means the program accepted the bytes and any other
//   error means it rejected them. This is the strict, on-chain ground truth.
//
// The three findings these tests pinned on #574 (trailing bytes, fee above
// the amount, extra accounts labelled multisig signers) were fixed before it
// merged; their tests now guard the fixes.

use super::*;
use proptest::prelude::*;
use solana_program_error::ProgramError;
use solana_sdk::pubkey::Pubkey;
use std::collections::BTreeMap;
use visualsign::encodings::decode_hex;

const UNPARSED: &str = "Token 2022 (unparsed)";

/// Tags whose decoding #574 added (26 = TransferFeeExtension, sub-tag 1 =
/// TransferCheckedWithFee).
const NEW_TAGS: [u8; 8] = [3, 4, 5, 7, 8, 12, 13, 26];

/// Tags the preset decoded before #574. Random-tag cases skip these, since
/// they are not under test here.
const PREEXISTING_TAGS: [u8; 7] = [6, 9, 10, 11, 14, 15, 44];

/// Tags whose instruction takes multisig signers after the fixed accounts,
/// with the number of fixed accounts.
const MULTISIG_TAGS: [(u8, usize); 6] = [(3, 3), (4, 3), (5, 2), (7, 3), (8, 3), (13, 4)];

// --- Rendering helpers (on top of mod.rs's `visualize`/`visualize_indices`) ---

fn program_key() -> Pubkey {
    Pubkey::new_from_array(spl_token_2022::id().to_bytes())
}

/// An account table: the program id at index 0, then `keys(n)`.
fn table(n: u8) -> Vec<Pubkey> {
    let mut t = vec![program_key()];
    t.extend(keys(n));
    t
}

fn unparsed(title: &str) -> bool {
    title == UNPARSED
}

/// The text of the first `label: text` row with this label.
fn row<'a>(rows: &'a [String], label: &str) -> Option<&'a str> {
    rows.iter()
        .find_map(|r| r.strip_prefix(label)?.strip_prefix(": "))
}

/// The "Additional Account N" rows: the accounts after the fixed ones.
fn extra_accounts(rows: &[String]) -> Vec<&str> {
    rows.iter()
        .filter(|r| r.starts_with("Additional Account "))
        .map(String::as_str)
        .collect()
}

/// Visualize `data` with `n` distinct accounts, in order. Returns the title,
/// the expanded rows and the account keys (without the program id).
fn render(data: &[u8], n: usize) -> (String, Vec<String>, Vec<Pubkey>) {
    let accounts = keys(n as u8);
    let (title, rows) = visualize(data.to_vec(), &accounts);
    (title, rows, accounts)
}

/// Visualize `data` with compiled account `indices` into `table`. Returns the
/// title and expanded rows, or the visualizer's error.
fn render_at(
    data: &[u8],
    table: &[Pubkey],
    indices: &[u8],
) -> Result<(String, Vec<String>), VisualSignError> {
    visualize_indices(data.to_vec(), table, indices.to_vec()).map(|(title, _, rows)| (title, rows))
}

// --- Encoders (layouts from token-2022 interface/src/instruction.rs) ---

fn amount_data(tag: u8, amount: u64) -> Vec<u8> {
    let mut d = vec![tag];
    d.extend_from_slice(&amount.to_le_bytes());
    d
}

fn amount_decimals_data(tag: u8, amount: u64, decimals: u8) -> Vec<u8> {
    let mut d = amount_data(tag, amount);
    d.push(decimals);
    d
}

fn with_fee_data(amount: u64, decimals: u8, fee: u64) -> Vec<u8> {
    let mut d = vec![26, 1];
    d.extend_from_slice(&amount.to_le_bytes());
    d.push(decimals);
    d.extend_from_slice(&fee.to_le_bytes());
    d
}

/// Exact on-chain length of each newly decoded instruction.
fn exact_len(tag: u8) -> usize {
    match tag {
        5 => 1,
        3 | 4 | 7 | 8 => 9,
        12 | 13 => 10,
        26 => 19,
        _ => unreachable!("not a new tag: {tag}"),
    }
}

/// The length the program requires, for the instructions it decodes with
/// `decode_instruction_data` (exact length). `None` where it ignores trailing
/// bytes: Revoke, and TransferCheckedWithFee, which it decodes with
/// `TransferFeeInstruction::unpack` like the preset does.
fn strict_len(tag: u8) -> Option<usize> {
    match tag {
        3 | 4 | 7 | 8 => Some(9),
        12 | 13 => Some(10),
        _ => None,
    }
}

// --- Oracles ---

#[derive(Debug, Clone, PartialEq)]
enum Want {
    Transfer {
        amount: u64,
    },
    Checked {
        amount: u64,
        decimals: u8,
        fee: Option<u64>,
    },
    Approve {
        amount: u64,
        decimals: Option<u8>,
    },
    Revoke,
    MintTo {
        amount: u64,
    },
    Burn {
        amount: u64,
    },
}

impl Want {
    /// Fixed accounts the instruction needs before any signers.
    fn fixed_accounts(&self) -> usize {
        match self {
            Want::Revoke => 2,
            Want::Checked { .. }
            | Want::Approve {
                decimals: Some(_), ..
            } => 4,
            _ => 3,
        }
    }

    fn decimals(&self) -> Option<u8> {
        match self {
            Want::Checked { decimals, .. } => Some(*decimals),
            Want::Approve { decimals, .. } => *decimals,
            _ => None,
        }
    }
}

/// What the interface crate decodes, restricted to the instructions #574
/// added. `None` for anything else, including the pre-existing decoders.
#[allow(deprecated)]
fn interface_decode(data: &[u8]) -> Option<Want> {
    if data.first() == Some(&26) {
        return match TransferFeeInstruction::unpack(&data[1..]) {
            Ok(TransferFeeInstruction::TransferCheckedWithFee {
                amount,
                decimals,
                fee,
            }) => Some(Want::Checked {
                amount,
                decimals,
                fee: Some(fee),
            }),
            _ => None,
        };
    }
    match TokenInstruction::unpack(data).ok()? {
        TokenInstruction::Transfer { amount } => Some(Want::Transfer { amount }),
        TokenInstruction::TransferChecked { amount, decimals } => Some(Want::Checked {
            amount,
            decimals,
            fee: None,
        }),
        TokenInstruction::Approve { amount } => Some(Want::Approve {
            amount,
            decimals: None,
        }),
        TokenInstruction::ApproveChecked { amount, decimals } => Some(Want::Approve {
            amount,
            decimals: Some(decimals),
        }),
        TokenInstruction::Revoke => Some(Want::Revoke),
        TokenInstruction::MintTo { amount } => Some(Want::MintTo { amount }),
        TokenInstruction::Burn { amount } => Some(Want::Burn { amount }),
        _ => None,
    }
}

/// Whether the Token-2022 program itself decodes `data` (see module docs).
fn program_decodes(data: &[u8]) -> bool {
    let program_id = spl_token_2022::id();
    match spl_token_2022::processor::Processor::process(&program_id, &[], data) {
        Ok(()) => panic!("no-account call succeeded for {data:02x?}"),
        Err(e) => matches!(e, ProgramError::NotEnoughAccountKeys),
    }
}

/// `amount` scaled by `decimals`, trailing zeros trimmed. Independent of
/// `format_token_amount`: string arithmetic, no division.
fn scaled(amount: u64, decimals: u8) -> String {
    let d = decimals as usize;
    let digits = format!("{amount:0>width$}", width = d + 1);
    let (whole, frac) = digits.split_at(digits.len() - d);
    let frac = frac.trim_end_matches('0');
    if frac.is_empty() {
        whole.to_string()
    } else {
        format!("{whole}.{frac}")
    }
}

fn hex(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect()
}

/// Role label -> account position, per instruction.
fn role_positions(want: &Want) -> BTreeMap<&'static str, usize> {
    let roles: &[(&str, usize)] = match want {
        Want::Transfer { .. } => &[("Source", 0), ("Destination", 1), ("Owner/Delegate", 2)],
        Want::Checked { .. } => &[
            ("Source", 0),
            ("Mint", 1),
            ("Destination", 2),
            ("Owner/Delegate", 3),
        ],
        Want::Approve { decimals: None, .. } => &[("Source", 0), ("Delegate", 1), ("Owner", 2)],
        Want::Approve { .. } => &[("Source", 0), ("Mint", 1), ("Delegate", 2), ("Owner", 3)],
        Want::Revoke => &[("Source", 0), ("Owner", 1)],
        Want::MintTo { .. } => &[
            ("Mint", 0),
            ("Destination Account", 1),
            ("Mint Authority", 2),
        ],
        Want::Burn { .. } => &[("Token Account", 0), ("Mint", 1), ("Authority", 2)],
    };
    roles.iter().copied().collect()
}

/// Assert that a decoded render shows exactly what `want` says.
fn assert_fields(title: &str, rows: &[String], want: &Want, data: &[u8]) {
    assert_eq!(
        row(rows, "Raw Data"),
        Some(hex(data).as_str()),
        "{data:02x?}"
    );
    match want {
        Want::Transfer { amount } => {
            assert_eq!(title, format!("Transfer: {amount} base units"));
            assert_eq!(row(rows, "Raw Amount"), Some(amount.to_string().as_str()));
            assert_eq!(
                row(rows, "Amount"),
                None,
                "no token amount without decimals"
            );
        }
        Want::MintTo { amount } => {
            assert_eq!(title, format!("Mint To: {amount} base units"));
            assert_eq!(row(rows, "Raw Amount"), Some(amount.to_string().as_str()));
            assert_eq!(row(rows, "Amount"), None);
        }
        Want::Burn { amount } => {
            assert_eq!(title, format!("Burn: {amount} base units"));
            assert_eq!(row(rows, "Raw Amount"), Some(amount.to_string().as_str()));
            assert_eq!(row(rows, "Amount"), None);
        }
        Want::Checked {
            amount,
            decimals,
            fee,
        } => {
            let name = if fee.is_some() {
                "Transfer Checked With Fee"
            } else {
                "Transfer Checked"
            };
            let shown = scaled(*amount, *decimals);
            assert_eq!(title, format!("{name}: {shown} tokens"));
            assert_eq!(row(rows, "Amount"), Some(shown.as_str()));
            assert_eq!(row(rows, "Raw Amount"), Some(amount.to_string().as_str()));
            assert_eq!(row(rows, "Decimals"), Some(decimals.to_string().as_str()));
            let want_fee = fee.map(|f| scaled(f, *decimals));
            assert_eq!(row(rows, "Fee").map(str::to_string), want_fee);
        }
        Want::Approve {
            amount,
            decimals: None,
        } => {
            assert_eq!(title, format!("Approve: {amount} base units"));
            assert_eq!(
                row(rows, "Amount"),
                Some(format!("{amount} base units").as_str())
            );
            assert_eq!(row(rows, "Raw Amount"), Some(amount.to_string().as_str()));
        }
        Want::Approve {
            amount,
            decimals: Some(d),
        } => {
            let shown = scaled(*amount, *d);
            assert_eq!(title, format!("Approve Checked: {shown}"));
            assert_eq!(row(rows, "Amount"), Some(shown.as_str()));
            assert_eq!(row(rows, "Decimals"), Some(d.to_string().as_str()));
        }
        Want::Revoke => assert_eq!(title, "Revoke Delegate"),
    }
}

/// Assert every role and signer row names the key at its account position.
fn assert_accounts(rows: &[String], want: &Want, shown_keys: &[Pubkey]) {
    for (label, pos) in role_positions(want) {
        assert_eq!(
            row(rows, label),
            Some(shown_keys[pos].to_string().as_str()),
            "{label} should be account {pos}"
        );
    }
    let fixed = want.fixed_accounts();
    let extra = extra_accounts(rows);
    assert_eq!(extra.len(), shown_keys.len().saturating_sub(fixed));
    for (i, shown) in extra.iter().enumerate() {
        let key = shown_keys[fixed + i];
        assert_eq!(*shown, format!("Additional Account {}: {key}", i + 1));
    }
}

/// A TransferCheckedWithFee whose fee exceeds its amount. The program's fee
/// is min(ceil(amount * bps / 10_000), max_fee) with bps <= 10_000, so it
/// rejects this with FeeMismatch. It checks that after reading the accounts,
/// so `program_decodes` can't see it; the preset refuses it up front.
fn fee_exceeds_amount(want: &Want) -> bool {
    matches!(want, Want::Checked { amount, fee: Some(fee), .. } if fee > amount)
}

/// Whether the preset should decode `data`, which `unpack` decodes as `want`,
/// given `n` accounts: `unpack`'s answer plus the program's exact-length and
/// fee rules and the preset's decimals and account-count bounds.
fn should_decode(want: &Want, data: &[u8], n: usize) -> bool {
    let decimals_ok = want.decimals().is_none_or(|d| d <= 18);
    let length_ok = strict_len(data[0]).is_none_or(|len| data.len() == len);
    let accounts_ok = match want {
        Want::Checked { .. } => n == 4,
        _ => n >= want.fixed_accounts(),
    };
    decimals_ok && length_ok && !fee_exceeds_amount(want) && accounts_ok
}

// --- Strategies ---

/// Amounts weighted to the edges.
fn amount() -> impl Strategy<Value = u64> {
    prop_oneof![
        Just(0u64),
        Just(1u64),
        Just(u64::MAX),
        Just(u64::MAX - 1),
        Just(10u64.pow(18)),
        any::<u64>(),
    ]
}

/// Decimals weighted to 0, 9, 18 (the preset's bound), 19 and 255.
fn decimals() -> impl Strategy<Value = u8> {
    prop_oneof![
        Just(0u8),
        Just(9u8),
        Just(18u8),
        Just(19u8),
        Just(255u8),
        any::<u8>(),
    ]
}

/// A well-formed encoding of one of the new instructions.
fn valid_new_instruction() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        (prop::sample::select(vec![3u8, 4, 7, 8]), amount()).prop_map(|(t, a)| amount_data(t, a)),
        (prop::sample::select(vec![12u8, 13]), amount(), decimals())
            .prop_map(|(t, a, d)| amount_decimals_data(t, a, d)),
        (amount(), decimals(), amount()).prop_map(|(a, d, f)| with_fee_data(a, d, f)),
        Just(vec![5u8]),
    ]
}

/// Arbitrary bytes behind a new tag, around the exact length: truncations,
/// off-by-one and trailing bytes. Also mutated valid encodings, and fully
/// arbitrary data (0..255 bytes, as in token-2022's
/// `test_instruction_unpack_proptest`) behind a random non-preexisting tag.
fn adversarial_data() -> impl Strategy<Value = Vec<u8>> {
    let around_exact = prop::sample::select(NEW_TAGS.to_vec()).prop_flat_map(|tag| {
        let n = exact_len(tag);
        let body = prop::collection::vec(any::<u8>(), 0..=n + 3);
        (Just(tag), body).prop_map(move |(tag, mut body)| {
            if tag == 26 && !body.is_empty() && body.len() % 2 == 0 {
                // Keep TransferCheckedWithFee's sub-tag in most cases.
                body[0] = 1;
            }
            let mut d = vec![tag];
            d.extend(body);
            d
        })
    });
    let truncated_valid = (
        valid_new_instruction(),
        any::<prop::sample::Index>(),
        any::<bool>(),
    )
        .prop_map(|(mut d, cut, extend)| {
            if extend {
                d.push(cut.index(256) as u8);
            } else {
                let keep = cut.index(d.len()).max(1);
                d.truncate(keep);
            }
            d
        });
    let random_tag = (any::<u8>(), prop::collection::vec(any::<u8>(), 0..255))
        .prop_filter("pre-existing decoder", |(t, _)| {
            !PREEXISTING_TAGS.contains(t)
        })
        .prop_map(|(t, body)| {
            let mut d = vec![t];
            d.extend(body);
            d
        });
    prop_oneof![
        4 => around_exact,
        3 => valid_new_instruction(),
        3 => truncated_valid,
        2 => random_tag,
        1 => Just(Vec::new()),
    ]
}

/// A valid new instruction with compiled account indices into `table(5)`
/// (0 is the program id, 1..=5 are accounts, 6.. are out of range). Mostly
/// `fixed..=fixed + 2` in-range indices, so the boundary cases (a checked
/// transfer with exactly 4, 5 or 6 accounts) come up often; otherwise
/// arbitrary indices: out of range, too few, too many, duplicates.
fn instruction_with_indices() -> impl Strategy<Value = (Vec<u8>, Vec<u8>)> {
    valid_new_instruction().prop_flat_map(|data| {
        let fixed = interface_decode(&data)
            .expect("valid encoding")
            .fixed_accounts();
        let indices = prop_oneof![
            3 => prop::collection::vec(0u8..=5, fixed..=fixed + 2),
            1 => prop::collection::vec(0u8..10, 0..=14),
        ];
        (Just(data), indices)
    })
}

proptest! {
    // Default 256 cases; override with PROPTEST_CASES=N.
    #![proptest_config(ProptestConfig::default())]

    /// Differential against the interface crate plus the program's exact
    /// length and fee rules (`should_decode`): the preset decodes exactly
    /// those (for the new instructions, decimals <= 18), with the same
    /// amount, decimals and fee, and otherwise shows the raw layout with the
    /// instruction's bytes.
    #[test]
    fn decodes_iff_interface_unpack_does(data in adversarial_data()) {
        let want = interface_decode(&data);
        let n = want.as_ref().map_or(4, Want::fixed_accounts);
        let (title, rows, accounts) = render(&data, n);
        match want {
            Some(want) if should_decode(&want, &data, n) => {
                prop_assert!(!unparsed(&title), "{data:02x?} should decode: {rows:?}");
                assert_fields(&title, &rows, &want, &data);
                assert_accounts(&rows, &want, &accounts);
            }
            _ => {
                prop_assert!(unparsed(&title), "{data:02x?} decoded as {title}");
                prop_assert_eq!(row(&rows, "Raw Data").map(str::to_string), Some(hex(&data)));
                prop_assert!(row(&rows, "Reason").is_some());
            }
        }
    }

    /// The preset never refuses data the Token-2022 program decodes, apart
    /// from its own decimals bound, and a fee above the amount, which the
    /// program rejects once it has the accounts.
    #[test]
    fn decodes_whatever_the_program_decodes(data in adversarial_data()) {
        prop_assume!(data.first().is_some_and(|t| NEW_TAGS.contains(t)));
        // Of the transfer-fee instructions only TransferCheckedWithFee is new.
        prop_assume!(data[0] != 26 || data.get(1) == Some(&1));
        prop_assume!(program_decodes(&data));
        let want = interface_decode(&data);
        // The interface crate is the more lenient of the two.
        let want = want.expect("program decodes, so unpack does");
        prop_assume!(want.decimals().is_none_or(|d| d <= 18));
        let (title, rows, _) = render(&data, want.fixed_accounts());
        if fee_exceeds_amount(&want) {
            prop_assert!(unparsed(&title), "{data:02x?}: fee above amount decoded as {title}");
        } else {
            prop_assert!(!unparsed(&title), "{data:02x?}: {rows:?}");
        }
    }

    /// The preset decodes nothing the program rejects. The program decodes
    /// these instructions with `decode_instruction_data`, which requires the
    /// exact length; `unpack` ignores trailing bytes. (Finding 1 on #574.)
    #[test]
    fn decodes_only_what_the_program_decodes(data in adversarial_data()) {
        prop_assume!(data.first().is_some_and(|t| NEW_TAGS.contains(t)));
        let want = interface_decode(&data);
        let n = want.as_ref().map_or(4, Want::fixed_accounts);
        let (title, _, _) = render(&data, n);
        if !unparsed(&title) {
            prop_assert!(program_decodes(&data), "{data:02x?} decoded as {title} but the program rejects it");
        }
    }

    /// Amounts: checked instructions show the amount scaled by the
    /// instruction's decimals and say "tokens"; unchecked ones show base
    /// units and no scaled amount. Decimals above 18, and a fee above the
    /// amount, are not decoded. A decoded fee is in the condensed view too.
    #[test]
    fn amounts_are_scaled_and_labelled(data in valid_new_instruction()) {
        let want = interface_decode(&data).expect("valid encoding");
        let (title, condensed, rows) = visualize_views(data.clone(), &keys(want.fixed_accounts() as u8));
        if fee_exceeds_amount(&want) {
            prop_assert!(unparsed(&title));
            prop_assert!(row(&rows, "Reason").is_some_and(|m| m.contains("exceeds amount")));
        } else if want.decimals().is_some_and(|d| d > 18) {
            prop_assert!(unparsed(&title));
            prop_assert!(row(&rows, "Reason").is_some_and(|m| m.contains("exceeds maximum supported value")));
        } else {
            prop_assert!(!unparsed(&title));
            assert_fields(&title, &rows, &want, &data);
            if let Want::Checked { fee: Some(fee), decimals, .. } = want {
                let shown = scaled(fee, decimals);
                prop_assert_eq!(row(&condensed, "Fee"), Some(shown.as_str()));
            }
            let units_in_title = title.contains("base units");
            let unchecked = want.decimals().is_none() && want != Want::Revoke;
            prop_assert_eq!(units_in_title, unchecked, "{}", title);
        }
    }

    /// TransferCheckedWithFee whose fee exceeds the amount. The program
    /// computes fee <= amount (basis points are capped at 10_000) and rejects
    /// any other fee with FeeMismatch, so the preview must not show it as a
    /// normal transfer. (Finding 2 on #574.)
    #[test]
    fn fee_above_amount_is_refused_or_flagged(
        amount in 0u64..u64::MAX,
        decimals in 0u8..=18,
        excess in 1u64..=u64::MAX,
    ) {
        let fee = amount.saturating_add(excess);
        prop_assume!(fee > amount);
        let (title, rows, _) = render(&with_fee_data(amount, decimals, fee), 4);
        let flagged = rows.iter().any(|r| r.to_lowercase().contains("exceeds"));
        prop_assert!(unparsed(&title) || flagged, "fee {fee} > amount {amount} shown as {title}");
    }

    /// Accounts: arbitrary compiled account indices (out of range, too few,
    /// too many, duplicates, the program id itself). The render either fails
    /// closed (an unresolvable index), shows the raw layout (too few or, for
    /// checked transfers, too many), or names each role with the key at its
    /// own position. It never takes an address from another index.
    #[test]
    fn accounts_fail_closed_or_match_their_index(
        (data, indices) in instruction_with_indices(),
    ) {
        let table = table(5);
        let result = render_at(&data, &table, &indices);
        if indices.iter().any(|&i| usize::from(i) >= table.len()) {
            prop_assert!(result.is_err(), "unresolvable index must fail closed");
            return Ok(());
        }
        let (title, rows) = result.expect("all indices resolve");
        let want = interface_decode(&data).expect("valid encoding");
        let shown: Vec<Pubkey> = indices.iter().map(|&i| table[usize::from(i)]).collect();
        if should_decode(&want, &data, indices.len()) {
            prop_assert!(!unparsed(&title), "{data:02x?} with {} accounts: {rows:?}", indices.len());
            assert_accounts(&rows, &want, &shown);
        } else {
            prop_assert!(unparsed(&title), "{data:02x?} with {} accounts decoded as {title}", indices.len());
            let addresses: Vec<String> = table[1..].iter().map(Pubkey::to_string).collect();
            for r in &rows {
                let (label, value) = r.split_once(": ").unwrap_or((r, ""));
                prop_assert!(
                    !addresses.iter().any(|a| a == value),
                    "raw layout shows an address as {label}"
                );
            }
        }
    }

    /// Multisig: 0 to 11 (MAX_SIGNERS) accounts after the fixed ones are
    /// listed in order, one "Additional Account N" row each.
    #[test]
    fn multisig_signers_match_the_account_list(
        tag_fixed in prop::sample::select(MULTISIG_TAGS.to_vec()),
        signers in 0usize..=11,
        amt in amount(),
        dec in 0u8..=18,
    ) {
        let (tag, fixed) = tag_fixed;
        let data = match tag {
            5 => vec![5],
            13 => amount_decimals_data(13, amt, dec),
            _ => amount_data(tag, amt),
        };
        let want = interface_decode(&data).expect("valid encoding");
        prop_assert_eq!(want.fixed_accounts(), fixed);
        let (title, rows, accounts) = render(&data, fixed + signers);
        prop_assert!(!unparsed(&title));
        assert_accounts(&rows, &want, &accounts);
        prop_assert_eq!(extra_accounts(&rows).len(), signers);
    }

    /// More than 11 extra accounts: a multisig has at most 11 (MAX_SIGNERS)
    /// signers, so at least one of them is not a signer, and the program
    /// ignores accounts that don't match one. No row may label them signers;
    /// each is listed as an additional account. (Finding 3 on #574.)
    #[test]
    fn more_than_eleven_extra_accounts_are_not_labelled_signers(
        tag_fixed in prop::sample::select(MULTISIG_TAGS.to_vec()),
        signers in 12usize..=20,
    ) {
        let (tag, fixed) = tag_fixed;
        let data = match tag {
            5 => vec![5],
            13 => amount_decimals_data(13, 1, 6),
            _ => amount_data(tag, 1),
        };
        let (title, rows, _) = render(&data, fixed + signers);
        prop_assert!(!unparsed(&title));
        for r in &rows {
            let (label, _) = r.split_once(": ").unwrap_or((r, ""));
            prop_assert!(!label.contains("Signer"), "{r}");
        }
        prop_assert_eq!(extra_accounts(&rows).len(), signers);
    }

    /// Instructions the preset doesn't decode: core instructions 16-25
    /// (InitializeAccount2, SyncNative, InitializeAccount3, InitializeMultisig2,
    /// InitializeMint2, GetAccountDataSize, InitializeImmutableOwner,
    /// AmountToUiAmount, UiAmountToAmount, InitializeMintCloseAuthority),
    /// extensions (the class of Trail of Bits' "Ok returned for malformed
    /// extension data") and unknown tags, well formed or not, are never
    /// decoded as a known instruction, and their raw bytes are shown.
    #[test]
    fn undecoded_core_extension_and_unknown_tags_show_raw_data(
        tag in (16u8..=255).prop_filter("decoded", |t| !NEW_TAGS.contains(t) && !PREEXISTING_TAGS.contains(t)),
        sub in any::<u8>(),
        body in prop::collection::vec(any::<u8>(), 0..64),
        n in 0usize..6,
    ) {
        let mut data = vec![tag, sub];
        data.extend(body);
        let (title, rows, _) = render(&data, n);
        prop_assert_eq!(&title, UNPARSED);
        prop_assert_eq!(row(&rows, "Raw Data").map(str::to_string), Some(hex(&data)));
        prop_assert!(row(&rows, "Reason").is_some());
    }

    /// TransferFeeExtension sub-instructions other than
    /// TransferCheckedWithFee (InitializeTransferFeeConfig, Withdraw...,
    /// Harvest..., SetTransferFee, unknown), with arbitrary bodies, stay raw.
    #[test]
    fn other_transfer_fee_instructions_stay_raw(
        sub in any::<u8>().prop_filter("TransferCheckedWithFee", |s| *s != 1),
        body in prop::collection::vec(any::<u8>(), 0..80),
    ) {
        let mut data = vec![26, sub];
        data.extend(body);
        let (title, rows, _) = render(&data, 4);
        prop_assert_eq!(&title, UNPARSED);
        prop_assert_eq!(row(&rows, "Raw Data").map(str::to_string), Some(hex(&data)));
    }
}

// --- Examples ---

/// Hand-pinned amounts, independent of `scaled`.
#[test]
fn pinned_amount_literals() {
    for (amount, decimals, shown) in [
        (u64::MAX, 0, "18446744073709551615"),
        (u64::MAX, 9, "18446744073.709551615"),
        (u64::MAX, 18, "18.446744073709551615"),
        (0, 9, "0"),
        (1, 18, "0.000000000000000001"),
        (1_500_000, 6, "1.5"),
    ] {
        let (title, _, _) = render(&amount_decimals_data(12, amount, decimals), 4);
        assert_eq!(title, format!("Transfer Checked: {shown} tokens"));
        assert_eq!(scaled(amount, decimals), shown);
    }
    let (title, _, _) = render(&amount_decimals_data(12, 1, 255), 4);
    assert!(unparsed(&title), "decimals 255 must not be decoded");
}

/// The concrete repro for `decodes_only_what_the_program_decodes`: each new
/// instruction at its exact length decodes (and the program agrees), and
/// with one byte fewer it does not (and the program agrees).
#[test]
fn exact_and_short_lengths_agree_with_the_program() {
    for data in [
        amount_data(3, 7),
        amount_data(4, 7),
        vec![5],
        amount_data(7, 7),
        amount_data(8, 7),
        amount_decimals_data(12, 7, 6),
        amount_decimals_data(13, 7, 6),
        with_fee_data(7, 6, 1),
    ] {
        assert_eq!(data.len(), exact_len(data[0]));
        let want = interface_decode(&data).unwrap();
        let (title, _, _) = render(&data, want.fixed_accounts());
        assert!(!unparsed(&title), "{data:02x?}");
        assert!(program_decodes(&data), "{data:02x?}");
        if data.len() > 1 {
            let short = &data[..data.len() - 1];
            let (title, _, _) = render(short, want.fixed_accounts());
            assert!(unparsed(&title), "{short:02x?} decoded as {title}");
            assert!(!program_decodes(short), "{short:02x?}");
        }
    }
}

/// One trailing byte: rejected by the program, and not decoded, for the
/// instructions it decodes at their exact length (finding 1 on #574).
/// Revoke and TransferCheckedWithFee, which the program decodes with
/// trailing bytes, still decode.
#[test]
fn one_trailing_byte_is_not_decoded() {
    let mut decoded = Vec::new();
    for mut data in [
        amount_data(3, 7),
        amount_data(4, 7),
        amount_data(7, 7),
        amount_data(8, 7),
        amount_decimals_data(12, 7, 6),
        amount_decimals_data(13, 7, 6),
    ] {
        data.push(0);
        let want = interface_decode(&data).unwrap();
        let (title, _, _) = render(&data, want.fixed_accounts());
        assert!(!program_decodes(&data), "{data:02x?}");
        if !unparsed(&title) {
            decoded.push(format!("{} -> {title}", hex(&data)));
        }
    }
    assert!(
        decoded.is_empty(),
        "decoded but rejected on chain: {decoded:#?}"
    );
    for mut data in [vec![5], with_fee_data(7, 6, 1)] {
        data.push(0);
        let want = interface_decode(&data).unwrap();
        let (title, _, _) = render(&data, want.fixed_accounts());
        assert!(program_decodes(&data), "{data:02x?}");
        assert!(!unparsed(&title), "{data:02x?}");
    }
}

/// Amount 1 base unit, fee 2: the program always rejects this (FeeMismatch).
#[test]
fn fee_above_amount_example() {
    let (title, rows, _) = render(&with_fee_data(1, 0, 2), 4);
    assert!(unparsed(&title), "shown as {title} with {rows:?}");
}

/// A checked transfer with a 5th account (a transfer hook's or a multisig
/// signer) is not decoded, with either form; with exactly 4 it is.
/// Deterministic companion to `accounts_fail_closed_or_match_their_index`.
#[test]
fn checked_transfer_with_exactly_five_accounts_is_unparsed() {
    for data in [amount_decimals_data(12, 7, 6), with_fee_data(7, 6, 1)] {
        let (title, rows, _) = render(&data, 4);
        assert!(!unparsed(&title), "{data:02x?} with 4 accounts: {rows:?}");
        let (title, rows, _) = render(&data, 5);
        assert!(
            unparsed(&title),
            "{data:02x?} with 5 accounts decoded as {title}"
        );
        assert!(
            row(&rows, "Reason").is_some_and(|r| r.contains("extra accounts")),
            "{rows:?}"
        );
    }
}

/// Source equal to destination is a valid self-transfer on chain. The
/// preview shows the same address in both rows, as the instruction says.
#[test]
fn source_equal_to_destination_is_shown_as_is() {
    let table = table(3);
    for data in [amount_decimals_data(12, 5, 2), amount_data(3, 5)] {
        let want = interface_decode(&data).unwrap();
        let indices: Vec<u8> = match want {
            // source, mint, destination(=source), authority
            Want::Checked { .. } => vec![1, 2, 1, 3],
            // source, destination(=source), authority
            _ => vec![1, 1, 3],
        };
        let (title, rows) = render_at(&data, &table, &indices).unwrap();
        assert!(!unparsed(&title));
        assert_eq!(row(&rows, "Source"), Some(table[1].to_string().as_str()));
        assert_eq!(
            row(&rows, "Destination"),
            Some(table[1].to_string().as_str())
        );
    }
}

/// The preview cannot know a mint's decimals; the program checks them
/// (MintDecimalsMismatch). Whatever the instruction says is shown, with its
/// mint and decimals side by side, so a wrong pair is visible.
#[test]
fn mint_and_decimals_are_shown_as_the_instruction_says() {
    // USDC-like mint (6 decimals) paired with 9 decimals in the instruction.
    let data = amount_decimals_data(12, 1_000_000_000, 9);
    let (title, rows, accounts) = render(&data, 4);
    assert_eq!(title, "Transfer Checked: 1 tokens");
    assert_eq!(row(&rows, "Decimals"), Some("9"));
    assert_eq!(row(&rows, "Mint"), Some(accounts[1].to_string().as_str()));
    // The same bytes with the mint in the source position: roles follow
    // positions, not keys.
    let table = table(4);
    let (_, rows) = render_at(&data, &table, &[2, 1, 3, 4]).unwrap();
    assert_eq!(row(&rows, "Source"), Some(table[2].to_string().as_str()));
    assert_eq!(row(&rows, "Mint"), Some(table[1].to_string().as_str()));
}

/// A multisig signer that is also the source or destination keeps both
/// roles, each at its own position.
#[test]
fn signer_that_is_also_source_or_destination() {
    let table = table(3);
    // Transfer: source=1, destination=2, authority=3, signers=[1, 2].
    let (_, rows) = render_at(&amount_data(3, 9), &table, &[1, 2, 3, 1, 2]).unwrap();
    assert_eq!(row(&rows, "Source"), Some(table[1].to_string().as_str()));
    assert_eq!(
        row(&rows, "Destination"),
        Some(table[2].to_string().as_str())
    );
    assert_eq!(
        row(&rows, "Owner/Delegate"),
        Some(table[3].to_string().as_str())
    );
    assert_eq!(
        extra_accounts(&rows),
        vec![
            format!("Additional Account 1: {}", table[1]),
            format!("Additional Account 2: {}", table[2]),
        ]
    );
}

// --- p-token conformance corpus ---

#[derive(serde::Deserialize)]
struct Corpus {
    cases: Vec<CorpusCase>,
}

#[derive(serde::Deserialize)]
struct CorpusCase {
    fixture: String,
    data: String,
    accounts: Vec<CorpusAccount>,
}

#[derive(serde::Deserialize)]
struct CorpusAccount {
    index: u8,
}

fn load_corpus() -> Corpus {
    let path = format!(
        "{}/tests/fixtures/token_2022/ptoken_conformance_instructions.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn corpus_bytes(case: &CorpusCase) -> Vec<u8> {
    decode_hex(&case.data).unwrap_or_else(|e| panic!("{}: {e}", case.fixture))
}

/// The SPL Token (p-token) conformance fixtures' instruction data and
/// account lists, retargeted at Token-2022: no panic, and the differential
/// against the interface crate holds, including the fixtures' duplicate
/// accounts.
#[test]
fn ptoken_conformance_corpus_matches_interface_unpack() {
    let corpus = load_corpus();
    assert!(corpus.cases.len() > 150, "corpus shrank");
    let table = table(16);
    let mut decoded = 0;
    for case in &corpus.cases {
        let data = corpus_bytes(case);
        let indices: Vec<u8> = case.accounts.iter().map(|a| a.index + 1).collect();
        let shown: Vec<Pubkey> = indices.iter().map(|&i| table[usize::from(i)]).collect();
        let (title, rows) =
            render_at(&data, &table, &indices).unwrap_or_else(|e| panic!("{}: {e}", case.fixture));
        let new_tag = data.first().is_some_and(|t| NEW_TAGS.contains(t));
        match interface_decode(&data) {
            Some(want) if new_tag && should_decode(&want, &data, indices.len()) => {
                assert!(!unparsed(&title), "{}: {rows:?}", case.fixture);
                assert_fields(&title, &rows, &want, &data);
                assert_accounts(&rows, &want, &shown);
                decoded += 1;
            }
            _ if new_tag => assert!(unparsed(&title), "{}: {title}", case.fixture),
            _ => {}
        }
    }
    assert!(decoded > 30, "only {decoded} corpus cases decoded");
}

/// The same corpus against the program's strict decoding: every corpus
/// instruction the preset decodes, the program decodes too. (The corpus has
/// no trailing-byte cases; `decodes_only_what_the_program_decodes` does.)
#[test]
fn ptoken_conformance_corpus_matches_the_program() {
    let corpus = load_corpus();
    let table = table(16);
    let mut mismatches = Vec::new();
    for case in &corpus.cases {
        let data = corpus_bytes(case);
        if !data.first().is_some_and(|t| NEW_TAGS.contains(t)) {
            continue;
        }
        let indices: Vec<u8> = case.accounts.iter().map(|a| a.index + 1).collect();
        let (title, _) = render_at(&data, &table, &indices).unwrap();
        if !unparsed(&title) && !program_decodes(&data) {
            mismatches.push(format!("{}: {}", case.fixture, case.data));
        }
    }
    assert!(mismatches.is_empty(), "{mismatches:#?}");
}
