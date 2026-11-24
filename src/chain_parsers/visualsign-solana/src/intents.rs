//! Solana's entry to the NEAR Intents decoder ([`visualsign_intents`]).
//!
//! Under `raw_ed25519` a Solana wallet signs the `DefusePayload` bytes directly,
//! so intents arrive as the message itself. Not under `presets/`, which holds
//! program-id visualizers that `build.rs` registers.
//!
//! Behind the `intents` feature, off by default, because `near-sdk` links the C
//! library `secp256k1-sys`. Without it, an intent-shaped message is refused.
//!
//! Limitations, since a Solana wallet signs message bytes as they are:
//! - A 32-byte message can be a NEP-413, SEP-53 or TON Connect prehash; the
//!   message path refuses one unless it is all ASCII letters and digits.
//! - A message of 69 bytes or more can be a NEAR Intents `webauthn` signature;
//!   the message path flags it (see `is_webauthn_shaped`). About 3 in 8 long
//!   texts match, so presentation is the wallet's call. The flag covers the
//!   message path only, not other signing paths such as transactions.

use std::sync::Arc;

use visualsign::errors::VisualSignError;
use visualsign::registry::LayeredRegistry;
use visualsign::vsptrait::VisualSignOptions;
use visualsign_intents::{NearIntentsError, NearTokenRegistry, RenderedEnvelope};

/// Render a message as NEAR Intents, or `Ok(None)` when it is not a
/// `DefusePayload`. A payload that fails to render is an error, never text.
/// Assets resolve from the compiled-in seed table only (`SolanaMetadata` carries
/// no NEAR token mappings), so an unknown asset renders unresolved.
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
