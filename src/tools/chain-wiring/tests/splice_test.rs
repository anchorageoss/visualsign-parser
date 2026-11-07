//! `splice` swaps only the generated M1 block of a tracking issue.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use chain_wiring::{SpliceError, WIRING_END, WIRING_START, splice};

fn block(line: &str) -> String {
    format!("{WIRING_START}\n{line}\n{WIRING_END}\n")
}

fn body(inside: &str) -> String {
    format!(
        "## M0 Scope\n\n- [x] Proto slot claimed (ticked by hand)\n- [ ] Default-on decided\n\n## M1 Wiring\n\n{WIRING_START}\n{inside}\n{WIRING_END}\n\n## M2\n\n- [x] Encoded size capped\n"
    )
}

#[test]
fn hand_ticks_outside_the_block_are_kept() {
    let old = body("- [ ] stale wiring line");
    let new = splice(&old, &block("- [x] fresh wiring line")).unwrap();
    assert_eq!(new, body("- [x] fresh wiring line"));
    assert!(new.contains("- [x] Proto slot claimed (ticked by hand)"));
    assert!(new.contains("- [x] Encoded size capped"));
    assert!(!new.contains("stale"));
}

#[test]
fn stale_multi_line_block_is_replaced_and_idempotent() {
    let old = body("\n**Crate**\n\n- [ ] a\n- [ ] b\n\n_Checked: 0/2_");
    let fresh = block("- [x] a\n- [x] b");
    let once = splice(&old, &fresh).unwrap();
    assert_eq!(once, body("- [x] a\n- [x] b"));
    assert_eq!(splice(&once, &fresh).unwrap(), once);
}

#[test]
fn empty_block_from_the_issue_form_is_filled() {
    let form = format!("### Milestones\n\n{WIRING_START}\n{WIRING_END}\n\n## M2");
    let new = splice(&form, &block("- [x] a")).unwrap();
    assert_eq!(
        new,
        format!("### Milestones\n\n{WIRING_START}\n- [x] a\n{WIRING_END}\n\n## M2")
    );
}

#[test]
fn missing_markers_are_refused() {
    assert_eq!(
        splice("no markers here\n", &block("x")),
        Err(SpliceError::NoStart)
    );
    let no_end = format!("a\n{WIRING_START}\nb\n");
    assert_eq!(splice(&no_end, &block("x")), Err(SpliceError::NoEnd));
    let reversed = format!("{WIRING_END}\n{WIRING_START}\n");
    assert_eq!(splice(&reversed, &block("x")), Err(SpliceError::NoEnd));
    let twice = format!("{}{}", body("a"), body("b"));
    assert_eq!(splice(&twice, &block("x")), Err(SpliceError::Duplicate));
    // A marker quoted inside a line is not a marker line.
    let quoted = format!("see `{WIRING_START}`\n");
    assert_eq!(splice(&quoted, &block("x")), Err(SpliceError::NoStart));
}

#[test]
fn crlf_bodies_stay_crlf() {
    let old = body("- [ ] stale").replace('\n', "\r\n");
    let new = splice(&old, &block("- [x] a\n- [x] b")).unwrap();
    assert_eq!(new, body("- [x] a\n- [x] b").replace('\n', "\r\n"));
    assert!(
        !new.replace("\r\n", "").contains('\n'),
        "every line ends in CRLF"
    );
}

#[test]
fn trailing_newline_is_preserved_either_way() {
    let without = body("x").trim_end().to_string();
    let new = splice(&without, &block("y")).unwrap();
    assert!(!new.ends_with('\n'));
    let with = body("x");
    assert!(
        splice(&with, &block("y"))
            .unwrap()
            .ends_with("- [x] Encoded size capped\n")
    );
}
