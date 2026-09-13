//! Text safe to put in front of a signer.

/// Mark what cannot render rather than dropping it: deleting would let two
/// different values render identically, and a signer comparing them has to be
/// able to tell them apart.
///
/// The policy itself is [`visualsign::charset::elide_unsupported`], shared with
/// `visualsign-near`, which renders the same intents content reached through a
/// NEAR envelope.
pub(crate) fn charset_safe(text: &str) -> String {
    visualsign::charset::elide_unsupported(text)
}
