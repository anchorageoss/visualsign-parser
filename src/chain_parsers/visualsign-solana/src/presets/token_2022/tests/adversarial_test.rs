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
//   are lenient (they ignore trailing bytes), so they are not ground truth.
// - `program_decodes`: the Token-2022 program's own entrypoint,
//   `spl_token_2022::processor::Processor::process`, called with no accounts.
//   It decodes the data first and only then asks for accounts, so
//   `NotEnoughAccountKeys` means the program accepted the bytes and any other
//   error means it rejected them. This is the strict, on-chain ground truth.
//
// Tests marked `#[ignore = "..."]` pin findings on #574; they fail today.

use super::*;
use crate::idl::IdlRegistry;
use proptest::prelude::*;
use solana_parser::solana::structs::SolanaAccount;
use solana_sdk::instruction::CompiledInstruction;
use solana_sdk::pubkey::Pubkey;
use std::collections::BTreeMap;

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

// ── Rendering helpers ────────────────────────────────────────────────────────

struct Render {
    title: String,
    rows: Vec<(String, String)>,
}

impl Render {
    fn unparsed(&self) -> bool {
        self.title == UNPARSED
    }

    fn row(&self, label: &str) -> Option<&str> {
        self.rows
            .iter()
            .find(|(l, _)| l == label)
            .map(|(_, v)| v.as_str())
    }

    fn signers(&self) -> Vec<(String, String)> {
        self.rows
            .iter()
            .filter(|(l, _)| l.starts_with("Multisig Signer"))
            .cloned()
            .collect()
    }
}

fn program_key() -> Pubkey {
    Pubkey::new_from_array(spl_token_2022::id().to_bytes())
}

/// `n` distinct non-program keys.
fn distinct_keys(n: usize) -> Vec<Pubkey> {
    (1..=n)
        .map(|i| {
            let mut bytes = [0u8; 32];
            bytes[0] = 0xa0;
            bytes[1] = i as u8;
            Pubkey::new_from_array(bytes)
        })
        .collect()
}

/// Run `data` through the visualizer as one compiled instruction whose
/// account indices are `indices` into `keys` (program id at index 0).
fn render_indices(data: &[u8], keys: &[Pubkey], indices: &[u8]) -> Result<Render, VisualSignError> {
    let compiled = CompiledInstruction {
        program_id_index: 0,
        accounts: indices.to_vec(),
        data: data.to_vec(),
    };
    let sender = SolanaAccount {
        account_key: keys[0].to_string(),
        signer: false,
        writable: false,
    };
    let idl = IdlRegistry::new();
    let context = VisualizerContext::new(&sender, &compiled, keys, &idl, 0);
    let field = Token2022Visualizer.visualize_tx_commands(&context)?;
    let SignablePayloadField::PreviewLayout { preview_layout, .. } = field.signable_payload_field
    else {
        panic!("expected PreviewLayout");
    };
    let rows = preview_layout
        .expanded
        .expect("expanded layout")
        .fields
        .iter()
        .map(|f| {
            let f = &f.signable_payload_field;
            (f.label().to_string(), f.fallback_text().to_string())
        })
        .collect();
    Ok(Render {
        title: preview_layout.title.expect("title").text,
        rows,
    })
}

/// Run `data` with `n` distinct accounts, in order. Returns the render and
/// the account keys (without the program id).
fn render(data: &[u8], n: usize) -> (Render, Vec<Pubkey>) {
    let accounts = distinct_keys(n);
    let mut keys = vec![program_key()];
    keys.extend_from_slice(&accounts);
    let indices: Vec<u8> = (1..=n as u8).collect();
    let r = render_indices(data, &keys, &indices).expect("all accounts resolve");
    (r, accounts)
}

// ── Encoders (layouts from token-2022 interface/src/instruction.rs) ──────────

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

// ── Oracles ──────────────────────────────────────────────────────────────────

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
        Err(e) => format!("{e:?}") == "NotEnoughAccountKeys",
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
fn assert_fields(r: &Render, want: &Want, data: &[u8]) {
    assert_eq!(r.row("Raw Data"), Some(hex(data).as_str()), "{data:02x?}");
    match want {
        Want::Transfer { amount } => {
            assert_eq!(r.title, format!("Transfer: {amount} base units"));
            assert_eq!(r.row("Raw Amount"), Some(amount.to_string().as_str()));
            assert_eq!(r.row("Amount"), None, "no token amount without decimals");
        }
        Want::MintTo { amount } => {
            assert_eq!(r.title, format!("Mint To: {amount} base units"));
            assert_eq!(r.row("Raw Amount"), Some(amount.to_string().as_str()));
            assert_eq!(r.row("Amount"), None);
        }
        Want::Burn { amount } => {
            assert_eq!(r.title, format!("Burn: {amount} base units"));
            assert_eq!(r.row("Raw Amount"), Some(amount.to_string().as_str()));
            assert_eq!(r.row("Amount"), None);
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
            assert_eq!(r.title, format!("{name}: {shown} tokens"));
            assert_eq!(r.row("Amount"), Some(shown.as_str()));
            assert_eq!(r.row("Raw Amount"), Some(amount.to_string().as_str()));
            assert_eq!(r.row("Decimals"), Some(decimals.to_string().as_str()));
            let want_fee = fee.map(|f| scaled(f, *decimals));
            assert_eq!(r.row("Fee").map(str::to_string), want_fee);
        }
        Want::Approve {
            amount,
            decimals: None,
        } => {
            assert_eq!(r.title, format!("Approve: {amount} base units"));
            assert_eq!(
                r.row("Amount"),
                Some(format!("{amount} base units").as_str())
            );
            assert_eq!(r.row("Raw Amount"), Some(amount.to_string().as_str()));
        }
        Want::Approve {
            amount,
            decimals: Some(d),
        } => {
            let shown = scaled(*amount, *d);
            assert_eq!(r.title, format!("Approve Checked: {shown}"));
            assert_eq!(r.row("Amount"), Some(shown.as_str()));
            assert_eq!(r.row("Decimals"), Some(d.to_string().as_str()));
        }
        Want::Revoke => assert_eq!(r.title, "Revoke Delegate"),
    }
}

/// Assert every role and signer row names the key at its account position.
fn assert_accounts(r: &Render, want: &Want, shown_keys: &[Pubkey]) {
    for (label, pos) in role_positions(want) {
        assert_eq!(
            r.row(label),
            Some(shown_keys[pos].to_string().as_str()),
            "{label} should be account {pos}"
        );
    }
    let fixed = want.fixed_accounts();
    let signers = r.signers();
    assert_eq!(signers.len(), shown_keys.len().saturating_sub(fixed));
    for (i, (label, value)) in signers.iter().enumerate() {
        assert_eq!(label, &format!("Multisig Signer {}", i + 1));
        assert_eq!(value, &shown_keys[fixed + i].to_string());
    }
}

/// Whether the preset should decode `want` given `n` accounts.
fn should_decode(want: &Want, n: usize) -> bool {
    let decimals_ok = want.decimals().is_none_or(|d| d <= 18);
    let accounts_ok = match want {
        Want::Checked { .. } => n == 4,
        _ => n >= want.fixed_accounts(),
    };
    decimals_ok && accounts_ok
}

// ── Strategies ───────────────────────────────────────────────────────────────

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

proptest! {
    // Default 256 cases; override with PROPTEST_CASES=N.
    #![proptest_config(ProptestConfig::default())]

    /// Differential against the interface crate: the preset decodes exactly
    /// what `unpack` decodes (for the new instructions, decimals <= 18), with
    /// the same amount, decimals and fee, and otherwise shows the raw layout
    /// with the instruction's bytes.
    #[test]
    fn decodes_iff_interface_unpack_does(data in adversarial_data()) {
        let want = interface_decode(&data);
        let n = want.as_ref().map_or(4, Want::fixed_accounts);
        let (r, accounts) = render(&data, n);
        match want {
            Some(want) if should_decode(&want, n) => {
                prop_assert!(!r.unparsed(), "{data:02x?} should decode: {:?}", r.rows);
                assert_fields(&r, &want, &data);
                assert_accounts(&r, &want, &accounts);
            }
            _ => {
                prop_assert!(r.unparsed(), "{data:02x?} decoded as {}", r.title);
                prop_assert_eq!(r.row("Raw Data").map(str::to_string), Some(hex(&data)));
                prop_assert!(r.row("Reason").is_some());
            }
        }
    }

    /// The preset never refuses data the Token-2022 program decodes, apart
    /// from its own decimals bound.
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
        let (r, _) = render(&data, want.fixed_accounts());
        prop_assert!(!r.unparsed(), "{data:02x?}: {:?}", r.rows);
    }

    /// FINDING: the preset decodes instructions the program rejects. The
    /// program decodes these instructions with `decode_instruction_data`,
    /// which requires the exact length; `unpack` ignores trailing bytes.
    #[test]
    #[ignore = "finding on #574: trailing bytes after Transfer/TransferChecked/Approve/ApproveChecked/MintTo/Burn are decoded, but the Token-2022 program rejects them (InvalidInstructionData)"]
    fn decodes_only_what_the_program_decodes(data in adversarial_data()) {
        prop_assume!(data.first().is_some_and(|t| NEW_TAGS.contains(t)));
        let want = interface_decode(&data);
        let n = want.as_ref().map_or(4, Want::fixed_accounts);
        let (r, _) = render(&data, n);
        if !r.unparsed() {
            prop_assert!(program_decodes(&data), "{data:02x?} decoded as {} but the program rejects it", r.title);
        }
    }

    /// Amounts: checked instructions show the amount scaled by the
    /// instruction's decimals and say "tokens"; unchecked ones show base
    /// units and no scaled amount. Decimals above 18 are not decoded.
    #[test]
    fn amounts_are_scaled_and_labelled(data in valid_new_instruction()) {
        let want = interface_decode(&data).expect("valid encoding");
        let (r, _) = render(&data, want.fixed_accounts());
        if want.decimals().is_some_and(|d| d > 18) {
            prop_assert!(r.unparsed());
            prop_assert!(r.row("Reason").is_some_and(|m| m.contains("exceeds maximum supported value")));
        } else {
            prop_assert!(!r.unparsed());
            assert_fields(&r, &want, &data);
            let units_in_title = r.title.contains("base units");
            let unchecked = want.decimals().is_none() && want != Want::Revoke;
            prop_assert_eq!(units_in_title, unchecked, "{}", r.title);
        }
    }

    /// FINDING: TransferCheckedWithFee whose fee exceeds the amount. The
    /// program computes fee <= amount (basis points are capped at 10_000) and
    /// rejects any other fee with FeeMismatch, but the preview shows it as
    /// is.
    #[test]
    #[ignore = "finding on #574: TransferCheckedWithFee with fee > amount is shown as a normal transfer, neither refused nor flagged"]
    fn fee_above_amount_is_refused_or_flagged(
        amount in 0u64..u64::MAX,
        decimals in 0u8..=18,
        excess in 1u64..=u64::MAX,
    ) {
        let fee = amount.saturating_add(excess);
        prop_assume!(fee > amount);
        let (r, _) = render(&with_fee_data(amount, decimals, fee), 4);
        let flagged = r.rows.iter().any(|(_, v)| v.to_lowercase().contains("exceeds"));
        prop_assert!(r.unparsed() || flagged, "fee {fee} > amount {amount} shown as {}", r.title);
    }

    /// Accounts: arbitrary compiled account indices (out of range, too few,
    /// duplicates, the program id itself). The render either fails closed
    /// (an unresolvable index), shows the raw layout (too few or, for checked
    /// transfers, too many), or names each role with the key at its own
    /// position. It never takes an address from another index.
    #[test]
    fn accounts_fail_closed_or_match_their_index(
        data in valid_new_instruction(),
        indices in prop::collection::vec(0u8..10, 0..=14),
    ) {
        // Index 0 is the program id; 1..=5 are accounts; 6.. are out of range.
        let mut keys = vec![program_key()];
        keys.extend(distinct_keys(5));
        let result = render_indices(&data, &keys, &indices);
        if indices.iter().any(|&i| usize::from(i) >= keys.len()) {
            prop_assert!(result.is_err(), "unresolvable index must fail closed");
            return Ok(());
        }
        let r = result.expect("all indices resolve");
        let want = interface_decode(&data).expect("valid encoding");
        let shown: Vec<Pubkey> = indices.iter().map(|&i| keys[usize::from(i)]).collect();
        if should_decode(&want, indices.len()) {
            prop_assert!(!r.unparsed());
            assert_accounts(&r, &want, &shown);
        } else {
            prop_assert!(r.unparsed());
            let addresses: Vec<String> = keys[1..].iter().map(Pubkey::to_string).collect();
            for (label, value) in &r.rows {
                prop_assert!(
                    !addresses.contains(value),
                    "raw layout shows an address as {label}"
                );
            }
        }
    }

    /// Multisig: 0 to 11 (MAX_SIGNERS) signer accounts are listed in order,
    /// one row each, after the fixed accounts.
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
        let (r, accounts) = render(&data, fixed + signers);
        prop_assert!(!r.unparsed());
        assert_accounts(&r, &want, &accounts);
        prop_assert_eq!(r.signers().len(), signers);
    }

    /// FINDING: more than 11 extra accounts are all labelled multisig
    /// signers. A multisig has at most 11 (MAX_SIGNERS) signers, so at least
    /// one of them is not. The program does not refuse the instruction: it
    /// ignores accounts that do not match a multisig signer.
    #[test]
    #[ignore = "finding on #574: more than MAX_SIGNERS (11) extra accounts are all labelled 'Multisig Signer N'"]
    fn more_than_eleven_signers_are_not_all_labelled_signers(
        tag_fixed in prop::sample::select(MULTISIG_TAGS.to_vec()),
        signers in 12usize..=20,
    ) {
        let (tag, fixed) = tag_fixed;
        let data = match tag {
            5 => vec![5],
            13 => amount_decimals_data(13, 1, 6),
            _ => amount_data(tag, 1),
        };
        let (r, _) = render(&data, fixed + signers);
        prop_assert!(r.unparsed() || r.signers().len() <= 11, "{} signer rows", r.signers().len());
    }

    /// Extension and unknown instructions, well formed or not (the class of
    /// Trail of Bits' "Ok returned for malformed extension data"), are never
    /// decoded as a known instruction, and their raw bytes are shown.
    #[test]
    fn extension_and_unknown_tags_show_raw_data(
        tag in (16u8..=255).prop_filter("decoded", |t| !NEW_TAGS.contains(t) && !PREEXISTING_TAGS.contains(t)),
        sub in any::<u8>(),
        body in prop::collection::vec(any::<u8>(), 0..64),
        n in 0usize..6,
    ) {
        let mut data = vec![tag, sub];
        data.extend(body);
        let (r, _) = render(&data, n);
        prop_assert_eq!(&r.title, UNPARSED);
        prop_assert_eq!(r.row("Raw Data").map(str::to_string), Some(hex(&data)));
        prop_assert!(r.row("Reason").is_some());
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
        let (r, _) = render(&data, 4);
        prop_assert_eq!(&r.title, UNPARSED);
        prop_assert_eq!(r.row("Raw Data").map(str::to_string), Some(hex(&data)));
    }
}

// ── Examples ─────────────────────────────────────────────────────────────────

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
        let (r, _) = render(&amount_decimals_data(12, amount, decimals), 4);
        assert_eq!(r.title, format!("Transfer Checked: {shown} tokens"));
        assert_eq!(scaled(amount, decimals), shown);
    }
    let (r, _) = render(&amount_decimals_data(12, 1, 255), 4);
    assert!(r.unparsed(), "decimals 255 must not be decoded");
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
        let (r, _) = render(&data, want.fixed_accounts());
        assert!(!r.unparsed(), "{data:02x?}");
        assert!(program_decodes(&data), "{data:02x?}");
        if data.len() > 1 {
            let short = &data[..data.len() - 1];
            let (r, _) = render(short, want.fixed_accounts());
            assert!(r.unparsed(), "{short:02x?} decoded as {}", r.title);
            assert!(!program_decodes(short), "{short:02x?}");
        }
    }
}

#[test]
#[ignore = "finding on #574: one trailing byte after these instructions is decoded, but the program rejects it with InvalidInstructionData"]
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
        let (r, _) = render(&data, want.fixed_accounts());
        assert!(!program_decodes(&data), "{data:02x?}");
        if !r.unparsed() {
            decoded.push(format!("{} -> {}", hex(&data), r.title));
        }
    }
    assert!(
        decoded.is_empty(),
        "decoded but rejected on chain: {decoded:#?}"
    );
}

#[test]
#[ignore = "finding on #574: TransferCheckedWithFee with fee > amount is shown as a normal transfer"]
fn fee_above_amount_example() {
    // amount 1 base unit, fee 2: the program always rejects this (FeeMismatch).
    let (r, _) = render(&with_fee_data(1, 0, 2), 4);
    assert!(r.unparsed(), "shown as {} with {:?}", r.title, r.rows);
}

/// Source equal to destination is a valid self-transfer on chain. The
/// preview shows the same address in both rows, as the instruction says.
#[test]
fn source_equal_to_destination_is_shown_as_is() {
    let mut keys = vec![program_key()];
    keys.extend(distinct_keys(3));
    for data in [amount_decimals_data(12, 5, 2), amount_data(3, 5)] {
        let want = interface_decode(&data).unwrap();
        let indices: Vec<u8> = match want {
            // source, mint, destination(=source), authority
            Want::Checked { .. } => vec![1, 2, 1, 3],
            // source, destination(=source), authority
            _ => vec![1, 1, 3],
        };
        let r = render_indices(&data, &keys, &indices).unwrap();
        assert!(!r.unparsed());
        assert_eq!(r.row("Source"), Some(keys[1].to_string().as_str()));
        assert_eq!(r.row("Destination"), Some(keys[1].to_string().as_str()));
    }
}

/// The preview cannot know a mint's decimals; the program checks them
/// (MintDecimalsMismatch). Whatever the instruction says is shown, with its
/// mint and decimals side by side, so a wrong pair is visible.
#[test]
fn mint_and_decimals_are_shown_as_the_instruction_says() {
    // USDC-like mint (6 decimals) paired with 9 decimals in the instruction.
    let data = amount_decimals_data(12, 1_000_000_000, 9);
    let (r, accounts) = render(&data, 4);
    assert_eq!(r.title, "Transfer Checked: 1 tokens");
    assert_eq!(r.row("Decimals"), Some("9"));
    assert_eq!(r.row("Mint"), Some(accounts[1].to_string().as_str()));
    // The same bytes with the mint in the source position: roles follow
    // positions, not keys.
    let mut keys = vec![program_key()];
    keys.extend(distinct_keys(4));
    let r = render_indices(&data, &keys, &[2, 1, 3, 4]).unwrap();
    assert_eq!(r.row("Source"), Some(keys[2].to_string().as_str()));
    assert_eq!(r.row("Mint"), Some(keys[1].to_string().as_str()));
}

/// A multisig signer that is also the source or destination keeps both
/// roles, each at its own position.
#[test]
fn signer_that_is_also_source_or_destination() {
    let mut keys = vec![program_key()];
    keys.extend(distinct_keys(3));
    // Transfer: source=1, destination=2, authority=3, signers=[1, 2].
    let r = render_indices(&amount_data(3, 9), &keys, &[1, 2, 3, 1, 2]).unwrap();
    assert_eq!(r.row("Source"), Some(keys[1].to_string().as_str()));
    assert_eq!(r.row("Destination"), Some(keys[2].to_string().as_str()));
    assert_eq!(r.row("Owner/Delegate"), Some(keys[3].to_string().as_str()));
    assert_eq!(
        r.signers(),
        vec![
            ("Multisig Signer 1".to_string(), keys[1].to_string()),
            ("Multisig Signer 2".to_string(), keys[2].to_string()),
        ]
    );
}

// ── p-token conformance corpus ───────────────────────────────────────────────

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
    (0..case.data.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&case.data[i..i + 2], 16).expect("hex"))
        .collect()
}

/// The SPL Token (p-token) conformance fixtures' instruction data and
/// account lists, retargeted at Token-2022: no panic, and the differential
/// against the interface crate holds, including the fixtures' duplicate
/// accounts.
#[test]
fn ptoken_conformance_corpus_matches_interface_unpack() {
    let corpus = load_corpus();
    assert!(corpus.cases.len() > 150, "corpus shrank");
    let mut keys = vec![program_key()];
    keys.extend(distinct_keys(16));
    let mut decoded = 0;
    for case in &corpus.cases {
        let data = corpus_bytes(case);
        let indices: Vec<u8> = case.accounts.iter().map(|a| a.index + 1).collect();
        let shown: Vec<Pubkey> = indices.iter().map(|&i| keys[usize::from(i)]).collect();
        let r = render_indices(&data, &keys, &indices)
            .unwrap_or_else(|e| panic!("{}: {e}", case.fixture));
        let new_tag = data.first().is_some_and(|t| NEW_TAGS.contains(t));
        match interface_decode(&data) {
            Some(want) if new_tag && should_decode(&want, indices.len()) => {
                assert!(!r.unparsed(), "{}: {:?}", case.fixture, r.rows);
                assert_fields(&r, &want, &data);
                assert_accounts(&r, &want, &shown);
                decoded += 1;
            }
            _ if new_tag => assert!(r.unparsed(), "{}: {}", case.fixture, r.title),
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
    let mut keys = vec![program_key()];
    keys.extend(distinct_keys(16));
    let mut mismatches = Vec::new();
    for case in &corpus.cases {
        let data = corpus_bytes(case);
        if !data.first().is_some_and(|t| NEW_TAGS.contains(t)) {
            continue;
        }
        let indices: Vec<u8> = case.accounts.iter().map(|a| a.index + 1).collect();
        let r = render_indices(&data, &keys, &indices).unwrap();
        if !r.unparsed() && !program_decodes(&data) {
            mismatches.push(format!("{}: {}", case.fixture, case.data));
        }
    }
    assert!(mismatches.is_empty(), "{mismatches:#?}");
}
