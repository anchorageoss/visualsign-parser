//! The detector on a slice of #625's real diff (`testdata/pr625.diff`): the
//! Solana intermediate schema v4 -> v5 bump plus `max_fee_lamports` in the
//! proto, alongside changes that must not fire (docs, display fixtures,
//! converter code).

use std::collections::BTreeMap;

use wallet_integration::body;
use wallet_integration::diff::{self, Side};
use wallet_integration::rules::{self, RULES};

const PR625_DIFF: &str = include_str!("../testdata/pr625.diff");
const INTERMEDIATE: &str = "src/chain_parsers/visualsign-solana/src/intermediate.rs";

/// `mod tests` line of intermediate.rs at #625's merge (head) and first parent (base).
fn test_start(path: &str, side: Side) -> Option<u32> {
    (path == INTERMEDIATE).then_some(match side {
        Side::Head => 1353,
        Side::Base => 1301,
    })
}

#[test]
fn pr625_fires_the_expected_rules() {
    let files = diff::parse(PR625_DIFF);
    assert_eq!(files.len(), 7);
    let found = rules::detect(&files, RULES, &test_start);

    let mut by_rule: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for f in &found {
        by_rule.entry(f.rule).or_default().push(f.path.clone());
    }
    assert_eq!(
        by_rule,
        BTreeMap::from([
            ("proto", vec!["proto/parser/parser.proto".to_owned()]),
            (
                "generated-proto",
                vec![
                    "src/generated/src/generated/descriptor.bin".to_owned(),
                    "src/generated/src/generated/parser.rs".to_owned(),
                ]
            ),
            ("schema-version", vec![INTERMEDIATE.to_owned()]),
            ("intermediate-types", vec![INTERMEDIATE.to_owned()]),
        ])
    );

    // The version bump is reported on the head-side line of the new constant.
    let schema = found
        .iter()
        .find(|f| f.rule == "schema-version")
        .map(ToString::to_string);
    assert_eq!(
        schema.as_deref().and_then(|s| s.split(": ").next()),
        Some("[schema-version] src/chain_parsers/visualsign-solana/src/intermediate.rs:73 (+1 more line(s))")
    );
    let proto = found.iter().find(|f| f.rule == "proto").map(|f| f.first);
    assert_eq!(proto, Some(Some((Side::Head, 172))));
}

#[test]
fn pr625_test_module_edits_alone_do_not_fire_intermediate_types() {
    let files: Vec<_> = diff::parse(PR625_DIFF)
        .into_iter()
        .filter(|f| f.path == INTERMEDIATE)
        .map(|mut f| {
            f.lines
                .retain(|l| l.number >= test_start(INTERMEDIATE, l.side).unwrap_or(u32::MAX));
            f
        })
        .collect();
    assert!(!files[0].lines.is_empty(), "#625 does edit the tests");
    assert!(rules::detect(&files, RULES, &test_start).is_empty());
}

/// The example section for #625 (see CONTRIBUTING.md) parses.
#[test]
fn pr625_example_section_parses() {
    let section = body::parse_section(include_str!("../testdata/pr625-section.md"))
        .unwrap_or_else(|p| panic!("{p:?}"));
    assert_eq!(section.requires.len(), 2);
    assert_eq!(section.deploy_order.len(), 1);
}
