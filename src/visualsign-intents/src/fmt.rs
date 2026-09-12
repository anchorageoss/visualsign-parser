//! Text safe to put in front of a signer.

const ELIDED: char = '?';

/// Mark what cannot render rather than dropping it: deleting would let two
/// different values render identically, and a signer comparing them has to be
/// able to tell them apart.
///
/// `visualsign-near`, `visualsign-solana` and `visualsign-ethereum` each carry
/// their own copy for their own rendering. Consolidating all four into
/// `visualsign` core is a follow-up; it is kept out of this change so the
/// extraction stays reviewable as a move.
pub(crate) fn charset_safe(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c == ' ' || (c.is_ascii_graphic() && c != '\\') {
                c
            } else {
                ELIDED
            }
        })
        .collect()
}
