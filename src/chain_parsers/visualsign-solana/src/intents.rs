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

#[cfg(feature = "intents")]
use std::sync::Arc;

#[cfg(feature = "intents")]
use visualsign::errors::VisualSignError;
#[cfg(feature = "intents")]
use visualsign::registry::LayeredRegistry;
#[cfg(feature = "intents")]
use visualsign::vsptrait::VisualSignOptions;
#[cfg(feature = "intents")]
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
#[cfg(feature = "intents")]
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
