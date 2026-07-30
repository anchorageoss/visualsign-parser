//! Seeded NEP-141 token table + amount formatting.

use visualsign::registry::LayeredRegistry;

use super::{NearTokenRegistry, TokenMeta};

/// Compiled-in mainnet seeds: `(asset_id, symbol, decimals)`.
///
/// Each entry MUST be verified against the token contract's `ft_metadata`
/// before being added; a wrong `decimals` silently misrenders amounts, so
/// anything not confidently verified is omitted and falls back to the raw
/// asset id. `wrap.near` is the canonical wrapped-NEAR contract (24 decimals,
/// matching native NEAR).
const SEEDS: &[(&str, &str, u8)] = &[("nep141:wrap.near", "wNEAR", 24)];

/// Resolve an asset id to its metadata: request-scoped override layer first
/// (via the registry), then the compiled-in seed table.
pub(crate) fn resolve(
    asset_id: &str,
    registry: &LayeredRegistry<NearTokenRegistry>,
) -> Option<TokenMeta> {
    if let Some(meta) = registry.lookup(|r| r.by_asset_id.get(asset_id).cloned()) {
        return Some(meta);
    }
    SEEDS
        .iter()
        .find(|(id, _, _)| *id == asset_id)
        .map(|(_, symbol, decimals)| TokenMeta {
            symbol: (*symbol).to_string(),
            decimals: *decimals,
        })
}

/// Format `units / 10^decimals` as an exact decimal string, trailing zeros
/// trimmed (e.g. `1_500_000` @ 6 -> `"1.5"`, `0` -> `"0"`).
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
    fn unknown_token_is_none() {
        assert!(resolve("nep141:not-a-real-token.near", &empty()).is_none());
    }

    #[test]
    fn format_with_decimals_trims() {
        assert_eq!(format_units(1_500_000, 6), "1.5");
        assert_eq!(format_units(0, 6), "0");
    }
}
