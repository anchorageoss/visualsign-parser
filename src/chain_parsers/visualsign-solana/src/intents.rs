//! Solana's entry to the NEAR Intents decoder.
//!
//! A Solana wallet signs a `DefusePayload` directly under the `raw_ed25519`
//! standard, where the signature covers the message bytes themselves. So the
//! intents arrive as the message, with no framing around them -- unlike NEAR,
//! where they arrive inside a NEP-413 envelope or an `execute_intents` call.
//!
//! The decoding is [`visualsign_intents`], shared with every chain that can
//! carry a payload. What is Solana-specific is only recognizing one here.
//!
//! Deliberately not under `presets/`. In this crate that directory means
//! instruction visualizers dispatched by program id -- `build.rs` scans it and
//! registers a `<Name>Visualizer` from each subdirectory. Intents arrive in an
//! off-chain message, never as an instruction, so they are reached from the
//! message path instead and would be registered against a program that never
//! matches.
//!
//! Behind the `intents` feature, off by default: `visualsign-intents` depends
//! on `near-sdk`, which pulls `near-crypto` -> `secp256k1-sys`, a C library
//! that a `--no-default-features` build of this crate otherwise avoids
//! entirely. Without the feature, a message shaped
//! like an intents payload is refused rather than shown as text (see
//! `message_to_visual_sign_payload`).
//!
//! A remaining limitation: a Solana wallet signs message bytes as they are, so
//! a 32-byte message can be the SHA-256 prehash a NEP-413, SEP-53 or TON
//! Connect verifier checks. The message path refuses a 32-byte message unless
//! it is all ASCII letters and digits; that bounds the risk, but only the
//! wallet, which sees what the dApp asked for, can rule it out.

use std::sync::Arc;

use visualsign::errors::VisualSignError;
use visualsign::registry::LayeredRegistry;
use visualsign::vsptrait::VisualSignOptions;
use visualsign_intents::{NearIntentsError, NearTokenRegistry, RenderedEnvelope};

/// Render a message as NEAR Intents, or `Ok(None)` when it is not one.
///
/// `Ok(None)` is reserved for a message that is not a `DefusePayload` at all,
/// which is the ordinary case -- a sign-in challenge, a note. A message that is
/// intents but fails to render is an error rather than a quiet fallback to text,
/// so a signer is never shown the raw JSON of something the parser partly
/// understood.
///
/// Assets resolve from the compiled-in seed table only. `ChainMetadata` is a
/// oneof, so a Solana request carries `SolanaMetadata` and has no field in which
/// to send NEAR token mappings; an asset outside the seed table renders as its
/// asset id, marked unresolved.
pub fn try_render(
    message: &[u8],
    options: &VisualSignOptions,
) -> Result<Option<RenderedEnvelope>, VisualSignError> {
    let registry = LayeredRegistry::new(Arc::new(NearTokenRegistry::default()));
    match visualsign_intents::try_render_single_intent(
        message,
        &registry,
        options,
        // Intents settle on NEAR mainnet. A Solana request cannot say otherwise:
        // its metadata is the Solana variant, which carries no NEAR network.
        visualsign_intents::network::SettlementNetwork::Mainnet,
    ) {
        Ok(rendered) => Ok(Some(rendered)),
        Err(NearIntentsError::InputNotJson(_)) => Ok(None),
        Err(other) => Err(VisualSignError::ConversionError(other.to_string())),
    }
}
