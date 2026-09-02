//! Seeded NEP-141 token table + amount formatting.

use visualsign::registry::LayeredRegistry;

use super::{NearTokenRegistry, TokenMeta};

/// Compiled-in mainnet seeds: `(asset_id, symbol, decimals)`.
///
/// Each entry MUST be verified against the token contract's `ft_metadata`
/// before being added; a wrong `decimals` silently misrenders amounts, so
/// anything not confidently verified is omitted and falls back to the raw
/// asset id. `wrap.near` is the canonical wrapped-NEAR contract (24 decimals,
/// matching native NEAR). The bridged entries below resolve through
/// `omni.bridge.near`'s own `get_token_id`/`get_native_token_id` registry via
/// `scripts/gen_near_token_seeds.sh`, not a hand-typed guess at the
/// `<chain>-<address>.omft.near` naming convention.
const SEEDS: &[(&str, &str, u8)] = &[
    ("nep141:wrap.near", "wNEAR", 24),
    (
        "nep141:a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48.factory.bridge.near",
        "USDC.e",
        6,
    ),
    (
        "nep141:dac17f958d2ee523a2206206994597c13d831ec7.factory.bridge.near",
        "USDT.e",
        6,
    ),
    ("nep141:eth.bridge.near", "ETH", 18),
];

/// Largest `decimals` [`format_units`] can scale by: `10^39` exceeds `u128`.
const MAX_DECIMALS: u8 = 38;

/// Metadata whose `decimals` [`format_units`] cannot scale by. Refusing it here
/// keeps that overflow unreachable, and renders the amount in its honest
/// unresolved form (raw base units plus the asset id) rather than scaled by a
/// wrapped-around divisor.
fn usable(meta: TokenMeta) -> Option<TokenMeta> {
    (meta.decimals <= MAX_DECIMALS).then_some(meta)
}

/// Whether `asset_id` has a compiled-in, verified entry in [`SEEDS`].
pub(crate) fn is_seeded(asset_id: &str) -> bool {
    SEEDS.iter().any(|(id, _, _)| *id == asset_id)
}

/// Resolve an asset id to its metadata: request-scoped override layer first
/// (via the registry), then the compiled-in seed table.
pub(crate) fn resolve(
    asset_id: &str,
    registry: &LayeredRegistry<NearTokenRegistry>,
) -> Option<TokenMeta> {
    if let Some(meta) = registry.lookup(|r| r.by_asset_id.get(asset_id).cloned()) {
        return usable(meta);
    }
    SEEDS
        .iter()
        .find(|(id, _, _)| *id == asset_id)
        .map(|(_, symbol, decimals)| TokenMeta {
            symbol: (*symbol).to_string(),
            decimals: *decimals,
            provenance: super::TokenProvenance::Seed,
        })
        .and_then(usable)
}

/// Format `units / 10^decimals` as an exact decimal string, trailing zeros
/// trimmed (e.g. `1_500_000` @ 6 -> `"1.5"`, `0` -> `"0"`).
///
/// `decimals` must be within [`MAX_DECIMALS`]; [`resolve`] is the only source
/// of registry-supplied values and refuses anything larger.
pub(crate) fn format_units(units: u128, decimals: u8) -> String {
    let scale = 10u128.pow(u32::from(decimals));
    let whole = units / scale;
    let frac = units % scale;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{frac:0width$}", width = decimals as usize);
    format!("{whole}.{}", frac_str.trim_end_matches('0'))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::presets::intents::TokenProvenance;
    use std::sync::Arc;
    use visualsign::registry::LayeredRegistry;

    fn empty() -> LayeredRegistry<NearTokenRegistry> {
        LayeredRegistry::new(Arc::new(NearTokenRegistry::default()))
    }

    #[test]
    fn seeded_token_resolves() {
        let meta = resolve("nep141:wrap.near", &empty()).expect("seeded");
        assert_eq!(meta.symbol, "wNEAR");
        assert_eq!(meta.decimals, 24);
    }

    #[test]
    fn seeded_bridged_token_resolves() {
        let meta = resolve(
            "nep141:a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48.factory.bridge.near",
            &empty(),
        )
        .expect("seeded");
        assert_eq!(meta.symbol, "USDC.e");
        assert_eq!(meta.decimals, 6);
    }

    #[test]
    fn unknown_token_is_none() {
        assert!(resolve("nep141:not-a-real-token.near", &empty()).is_none());
    }

    // `format_units` scales by `10^decimals`, which overflows `u128` above 38.
    // An override carrying such a value must not resolve: the amount then
    // renders as raw base units plus the asset id, rather than overflowing (or,
    // with overflow checks off, dividing by a wrapped-around scale and
    // misrendering the amount being signed).
    #[test]
    fn override_with_unscalable_decimals_does_not_resolve() {
        let asset_id = "nep141:broken.near";
        let mut request = NearTokenRegistry::default();
        request.by_asset_id.insert(
            asset_id.to_string(),
            TokenMeta {
                symbol: "BROKEN".to_string(),
                decimals: 39,
                provenance: TokenProvenance::Unsigned,
            },
        );
        let registry =
            LayeredRegistry::with_request(Arc::new(NearTokenRegistry::default()), request);
        assert!(resolve(asset_id, &registry).is_none());
    }

    #[test]
    fn override_at_the_decimals_bound_resolves() {
        let asset_id = "nep141:wide.near";
        let mut request = NearTokenRegistry::default();
        request.by_asset_id.insert(
            asset_id.to_string(),
            TokenMeta {
                symbol: "WIDE".to_string(),
                decimals: MAX_DECIMALS,
                provenance: TokenProvenance::Unsigned,
            },
        );
        let registry =
            LayeredRegistry::with_request(Arc::new(NearTokenRegistry::default()), request);
        let meta = resolve(asset_id, &registry).expect("resolves at the bound");
        assert_eq!(meta.decimals, MAX_DECIMALS);
        // The bound is exactly what `format_units` can scale by.
        assert_eq!(format_units(0, MAX_DECIMALS), "0");
    }

    #[test]
    fn is_seeded_matches_seeds_table() {
        assert!(is_seeded("nep141:wrap.near"));
        assert!(!is_seeded("nep141:not-a-real-token.near"));
    }

    #[test]
    fn format_with_decimals_trims() {
        assert_eq!(format_units(1_500_000, 6), "1.5");
        assert_eq!(format_units(0, 6), "0");
    }
}
