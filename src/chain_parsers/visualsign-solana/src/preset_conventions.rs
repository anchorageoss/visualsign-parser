//! Convention gate for Solana program visualizers.
//!
//! Every Anchor/IDL preset must surface the instruction discriminator as a
//! `"Discriminator"` field for every instruction it decodes, matching the
//! established Solana program-visualization convention.
//!
//! The gate is behavioural: for each preset with an embedded IDL
//! (`PRESET_IDLS`), it builds one synthetic instruction per IDL instruction
//! (its discriminator followed by zeroed args), runs the preset's own
//! visualizer on it, and requires a `"Discriminator"` field somewhere in the
//! result. A preset that only emits the field from a fallback arm fails here.
//!
//! Presets without an IDL JSON can't be exercised this way, so each one must be
//! listed in `NON_IDL_PRESETS` with the reason; a new preset directory that is
//! neither IDL-backed nor listed fails `every_preset_is_gated_or_listed`.

use crate::core::{InstructionVisualizer, VisualizerContext, preset_visualizers_by_name};
use crate::idl::IdlRegistry;
use solana_parser::solana::structs::SolanaAccount;
use solana_parser::{Idl, decode_idl_data, parse_instruction_with_idl};
use solana_sdk::instruction::CompiledInstruction;
use solana_sdk::pubkey::Pubkey;
use std::collections::BTreeSet;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Presets with no embedded IDL, so the behavioural gate can't build inputs for them.
const NON_IDL_PRESETS: &[&str] = &[
    // Native programs: no Anchor 8-byte discriminator (1-byte/native tags).
    "associated_token_account",
    "compute_budget",
    "spl_token",
    "stakepool",
    "system",
    "token_2022",
    // Catch-all for programs no preset recognizes.
    "unknown_program",
    // Hand-written decoder without an IDL JSON. It only surfaces the discriminator
    // in its fallback arm; the known instructions don't (tracked separately).
    "swig_wallet",
];

/// IDL presets that don't surface a Discriminator field yet. Remove an entry once
/// the preset is brought into line; `grandfathered_presets_still_fail` keeps this honest.
const GRANDFATHERED: &[&str] = &["jupiter_swap", "orca_whirlpool"];

/// Upper bound when searching for the zeroed-args length of an instruction. The
/// parser refuses longer call data ("exceeded max cursor length").
const MAX_ARGS_LEN: usize = 1024;

/// Accounts passed to every synthetic instruction, so account-count checks pass.
const ACCOUNT_COUNT: usize = 64;

/// Discriminator followed by the exact number of zero bytes the IDL args need.
/// All-zero args decode to zero numbers, empty vectors/strings, `None` options and
/// the first enum variant, so the length is found by binary search: too few bytes
/// fail to decode, too many fail with "extra unexpected bytes" (or the parser's
/// length cap).
fn zeroed_instruction_data(idl: &Idl, discriminator: &[u8]) -> Option<Vec<u8>> {
    let build = |len: usize| {
        let mut data = discriminator.to_vec();
        data.resize(discriminator.len() + len, 0);
        data
    };
    let (mut lo, mut hi) = (0, MAX_ARGS_LEN);
    while lo <= hi {
        let mid = lo + (hi - lo) / 2;
        let data = build(mid);
        match parse_instruction_with_idl(&data, "", idl) {
            Ok(_) => return Some(data),
            Err(e)
                if e.to_string().contains("extra unexpected bytes")
                    || e.to_string().contains("exceeded max cursor length") =>
            {
                hi = mid.checked_sub(1)?
            }
            Err(_) => lo = mid + 1,
        }
    }
    None
}

/// Whether the visualizer's view of `data` contains a Discriminator field, or
/// `None` if the visualizer returned an error.
fn discriminator_surfaced(
    visualizer: &dyn InstructionVisualizer,
    idl_registry: &IdlRegistry,
    program_id: Pubkey,
    data: Vec<u8>,
) -> Option<bool> {
    let mut account_keys = vec![program_id];
    account_keys.extend((0..ACCOUNT_COUNT).map(|_| Pubkey::new_unique()));
    let compiled = CompiledInstruction {
        program_id_index: 0,
        accounts: (1..=ACCOUNT_COUNT as u8).collect(),
        data,
    };
    let sender = SolanaAccount {
        account_key: account_keys[1].to_string(),
        signer: true,
        writable: true,
    };
    let context = VisualizerContext::new(&sender, &compiled, &account_keys, idl_registry, 0);

    let field = visualizer.visualize_tx_commands(&context).ok()?;
    let json = serde_json::to_string(&field.signable_payload_field).ok()?;
    Some(json.contains("\"Label\":\"Discriminator\""))
}

/// The first program ID the visualizer's config claims.
fn program_id_of(visualizer: &dyn InstructionVisualizer) -> Option<Pubkey> {
    let config = visualizer.get_config()?;
    config.data().programs.keys().next()?.parse().ok()
}

/// One preset's result: IDL instructions whose view lacks a Discriminator field, and
/// those the gate couldn't exercise (no zeroed encoding, or the visualizer errored).
struct PresetAudit {
    name: String,
    missing: Vec<String>,
    unchecked: Vec<String>,
}

/// Audit every IDL preset once; the tests below share the result.
fn audit() -> &'static [PresetAudit] {
    static AUDIT: OnceLock<Vec<PresetAudit>> = OnceLock::new();
    AUDIT.get_or_init(run_audit)
}

/// Outcome of rendering one IDL instruction through its preset's visualizer.
enum Outcome {
    Surfaced,
    Missing,
    Unchecked(&'static str),
}

/// Every (preset, instruction) pair is independent and the decode is slow in debug
/// builds, so the pairs are spread over a few threads. Each thread builds its own
/// visualizers; the decoded IDLs are shared.
fn run_audit() -> Vec<PresetAudit> {
    let presets: Vec<(&str, Idl)> = crate::PRESET_IDLS
        .iter()
        .map(|(name, json)| {
            let idl = decode_idl_data(json).unwrap_or_else(|e| panic!("{name}: {e}"));
            (*name, idl)
        })
        .collect();
    let jobs: Vec<(usize, usize)> = presets
        .iter()
        .enumerate()
        .flat_map(|(p, (_, idl))| (0..idl.instructions.len()).map(move |i| (p, i)))
        .collect();

    let next = AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let mut outcomes: Vec<(usize, usize, Outcome)> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let visualizers = preset_visualizers_by_name();
                    let idl_registry = IdlRegistry::new();
                    let mut done = Vec::new();
                    loop {
                        let Some(&(p, i)) = jobs.get(next.fetch_add(1, Ordering::Relaxed)) else {
                            return done;
                        };
                        let (name, idl) = &presets[p];
                        let (_, visualizer) = visualizers
                            .iter()
                            .find(|(n, _)| n == name)
                            .unwrap_or_else(|| {
                                panic!("{name}: IDL JSON without a preset visualizer")
                            });
                        let program_id = program_id_of(visualizer.as_ref())
                            .unwrap_or_else(|| panic!("{name}: no program id"));
                        let outcome = match idl.instructions[i]
                            .discriminator
                            .as_deref()
                            .filter(|d| !d.is_empty())
                        {
                            None => Outcome::Unchecked("no discriminator"),
                            Some(disc) => match zeroed_instruction_data(idl, disc) {
                                None => Outcome::Unchecked("no zeroed encoding"),
                                Some(data) => match discriminator_surfaced(
                                    visualizer.as_ref(),
                                    &idl_registry,
                                    program_id,
                                    data,
                                ) {
                                    Some(true) => Outcome::Surfaced,
                                    Some(false) => Outcome::Missing,
                                    None => Outcome::Unchecked("visualizer error"),
                                },
                            },
                        };
                        done.push((p, i, outcome));
                    }
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().unwrap_or_else(|_| panic!("audit worker panicked")))
            .collect()
    });
    outcomes.sort_by_key(|(p, i, _)| (*p, *i));

    let mut report: Vec<PresetAudit> = presets
        .iter()
        .map(|(name, _)| PresetAudit {
            name: name.to_string(),
            missing: Vec::new(),
            unchecked: Vec::new(),
        })
        .collect();
    for (p, i, outcome) in outcomes {
        let ix_name = &presets[p].1.instructions[i].name;
        match outcome {
            Outcome::Surfaced => {}
            Outcome::Missing => report[p].missing.push(ix_name.clone()),
            Outcome::Unchecked(why) => report[p].unchecked.push(format!("{ix_name} ({why})")),
        }
    }
    report
}

#[test]
fn idl_presets_surface_discriminator_for_every_instruction() {
    let failures: Vec<String> = audit()
        .iter()
        .filter(|a| !a.missing.is_empty() && !GRANDFATHERED.contains(&a.name.as_str()))
        .map(|a| format!("{}: {:?}", a.name, a.missing))
        .collect();
    assert!(
        failures.is_empty(),
        "these presets decode instructions without a \"Discriminator\" field; add \
         create_text_field(\"Discriminator\", ...) to every decoded instruction:\n{}",
        failures.join("\n")
    );
}

#[test]
fn every_idl_instruction_is_exercised() {
    // An instruction the gate can't build or render would silently escape it.
    let unchecked: Vec<String> = audit()
        .iter()
        .filter(|a| !a.unchecked.is_empty())
        .map(|a| format!("{}: {:?}", a.name, a.unchecked))
        .collect();
    assert!(
        unchecked.is_empty(),
        "the discriminator gate could not exercise these instructions:\n{}",
        unchecked.join("\n")
    );
}

#[test]
fn grandfathered_presets_still_fail() {
    for name in GRANDFATHERED {
        let entry = audit()
            .iter()
            .find(|a| a.name == *name)
            .unwrap_or_else(|| panic!("grandfathered preset '{name}' has no IDL; remove it"));
        assert!(
            !entry.missing.is_empty(),
            "'{name}' now surfaces the discriminator everywhere; remove it from GRANDFATHERED"
        );
    }
}

#[test]
fn every_preset_is_gated_or_listed() {
    let idl_presets: BTreeSet<&str> = crate::PRESET_IDLS.iter().map(|(n, _)| *n).collect();
    let unlisted: Vec<&str> = preset_visualizers_by_name()
        .iter()
        .map(|(n, _)| *n)
        .filter(|n| !idl_presets.contains(n) && !NON_IDL_PRESETS.contains(n))
        .collect();
    assert!(
        unlisted.is_empty(),
        "presets with no IDL JSON must be listed in NON_IDL_PRESETS with a reason: {unlisted:?}"
    );
    for name in NON_IDL_PRESETS {
        assert!(
            preset_visualizers_by_name().iter().any(|(n, _)| n == name),
            "NON_IDL_PRESETS entry '{name}' is not a preset; remove it"
        );
        assert!(
            !idl_presets.contains(name),
            "'{name}' has an IDL JSON now; remove it from NON_IDL_PRESETS so the gate covers it"
        );
    }
}
