//! The CI gate: every chain in this repo is fully wired, except the gaps
//! listed in `known-gaps.toml`, and every listed gap is still a gap (so the
//! list only shrinks). A chain with no section of its own inherits only the
//! repo-wide default-set gaps, the same rule `pr-check` applies to a new crate.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use chain_wiring::pr::{allowed_gaps, parse_known_gaps};
use chain_wiring::{Kind, Names, Rules, check_with};
use std::collections::BTreeSet;
use std::path::Path;

#[test]
fn every_chain_is_wired_except_known_gaps() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let rules = Rules::builtin().unwrap();
    let known = parse_known_gaps(include_str!("../known-gaps.toml")).unwrap();
    let chains = rules.discover_chains(&root).unwrap();
    assert!(
        chains.len() >= 6,
        "found only {chains:?} under src/chain_parsers"
    );

    let ids: BTreeSet<&str> = rules.touchpoints.iter().map(|t| t.id.as_str()).collect();
    let mut problems = Vec::new();
    for (chain, gaps) in &known {
        if !chains.contains(chain) {
            problems.push(format!(
                "known-gaps.toml: [{chain}] is not a chain under src/chain_parsers"
            ));
        }
        for id in gaps.keys().filter(|id| !ids.contains(id.as_str())) {
            problems.push(format!(
                "known-gaps.toml: [{chain}] {id} is not a touch point id"
            ));
        }
    }
    let repo_wide: Vec<&str> = rules
        .touchpoints
        .iter()
        .filter(|t| t.kind == Kind::DefaultSetEquality)
        .map(|t| t.id.as_str())
        .collect();
    for chain in &chains {
        let report = check_with(&rules, &root, &Names::new(chain, None).unwrap()).unwrap();
        let has_section = known.contains_key(chain);
        let allowed = allowed_gaps(chain, !has_section, &known, &repo_wide);
        let missing = report.missing_ids();
        for item in report
            .items
            .iter()
            .filter(|i| missing.contains(i.id.as_str()))
        {
            if !allowed.contains_key(&item.id) {
                problems.push(format!("{chain}: MISSING {} [{}]", item.text, item.id));
            }
        }
        // Inherited gaps belong to another chain's section; only a chain's own
        // entries can be stale.
        let own = allowed.keys().filter(|_| has_section);
        for id in own.filter(|id| !missing.contains(id.as_str())) {
            problems.push(format!(
                "{chain}: {id} is wired now; delete it from known-gaps.toml"
            ));
        }
    }
    assert!(
        problems.is_empty(),
        "chain wiring gate (run `cargo run -q -p chain-wiring -- check <chain>`):\n  {}",
        problems.join("\n  ")
    );
}
