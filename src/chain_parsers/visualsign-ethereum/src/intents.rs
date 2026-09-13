//! Ethereum's entry to the NEAR Intents decoder.
//!
//! An Ethereum-keyed identity signs a `DefusePayload` under ERC-191, so the
//! intents arrive as the personal-sign message itself. The signer_id in the
//! payload is the Ethereum address; the verifier recovers the key from the
//! signature rather than being told it.
//!
//! The decoding is [`visualsign_intents`], shared with every chain that can
//! carry a payload. What is Ethereum-specific is only recognizing one here.
//!
//! Not under `contracts/` or `protocols/`: those decode calldata against an ABI,
//! and an off-chain message has no calldata to decode.

use std::sync::Arc;

use visualsign::errors::VisualSignError;
use visualsign::registry::LayeredRegistry;
use visualsign::vsptrait::VisualSignOptions;
use visualsign_intents::{NearIntentsError, NearTokenRegistry, RenderedEnvelope};

/// Render a message as NEAR Intents, or `Ok(None)` when it is not one.
///
/// `Ok(None)` is reserved for a message that is not a `DefusePayload` at all,
/// which is the ordinary case -- a sign-in challenge, a terms acceptance. A
/// message that is intents but fails to render is an error rather than a quiet
/// fallback to text, so a signer is never shown the raw JSON of something the
/// parser partly understood.
///
/// Assets resolve from the compiled-in seed table only. `ChainMetadata` is a
/// oneof, so an Ethereum request carries `EthereumMetadata` and has no field in
/// which to send NEAR token mappings; an asset outside the seed table renders as
/// its asset id, marked unresolved.
pub fn try_render(
    message: &[u8],
    options: &VisualSignOptions,
) -> Result<Option<RenderedEnvelope>, VisualSignError> {
    let registry = LayeredRegistry::new(Arc::new(NearTokenRegistry::default()));
    match visualsign_intents::try_render_single_intent(
        message,
        &registry,
        options,
        // Intents settle on NEAR mainnet. An Ethereum request cannot say
        // otherwise: its metadata is the Ethereum variant, with no NEAR network.
        visualsign_intents::network::SettlementNetwork::Mainnet,
    ) {
        Ok(rendered) => Ok(Some(rendered)),
        Err(NearIntentsError::InputNotJson(_)) => Ok(None),
        Err(other) => Err(VisualSignError::ConversionError(other.to_string())),
    }
}
