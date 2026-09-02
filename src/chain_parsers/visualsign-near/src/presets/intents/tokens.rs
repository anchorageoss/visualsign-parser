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
///
/// # Symbols are origin-qualified, and unique
///
/// The on-chain `ft_metadata` symbol is not usable verbatim. Four distinct
/// assets report `{"symbol":"ETH","decimals":18}` -- Ethereum's ether via the
/// omni bridge and via the legacy rainbow bridge, plus ether bridged from Base
/// and from Arbitrum -- and four report `{"symbol":"USDC","decimals":6}`.
/// `SignablePayloadFieldAmountV2` carries only an amount and an abbreviation,
/// so seeding the raw symbol would render Base ether identically to mainnet
/// ether, and a swap of one for the other identically to its mirror.
///
/// So the bare symbol belongs to the asset native to its own chain, and
/// anything bridged from elsewhere carries an origin suffix (`ETH.base`,
/// `USDC.sol`). `.e` marks the legacy rainbow-bridge entries, the convention
/// the first stablecoin seeds already used. `every_seeded_symbol_is_unique`
/// enforces this: a new entry that collides fails the test rather than
/// silently making two assets look alike.
///
/// # Sourcing
///
/// `scripts/gen_near_token_seeds.sh` resolves ids through `omni.bridge.near`'s
/// own registry, rather than hand-typing the `<chain>-<address>.omft.near`
/// convention. Its output is a starting point, not the authority. Both of its
/// lookups answer with ids that real intents do not carry:
/// `get_native_token_id` gives `Eth` the legacy `eth.bridge.near` and
/// `Base`/`Arb`/`Pol` their `.omdep.near` deposit contracts, and
/// `get_token_id` gives Ethereum USDC/USDT their legacy `factory.bridge.near`
/// ids while returning nothing at all for Base or Arbitrum.
///
/// So every entry's `symbol`/`decimals` is confirmed against the token
/// contract's own `ft_metadata`, and its id rests on one of these:
///
/// - **observed traffic** -- `eth.omft.near` and `sol.omft.near` appear in
///   captured production envelopes, which is also what establishes
///   `<chain>.omft.near` as the intents-facing form for the entries beside
///   them;
/// - **the bridge registry** -- `get_token_id` maps the canonical Solana USDC
///   and USDT mints onto the two `sol-*` ids, and `get_native_token_id` names
///   `nbtc.bridge.near` for `Btc`;
/// - **a self-describing id** -- the two `eth-0x...` ids embed the source-chain
///   contract address, so it can be checked against Ethereum directly.
///
/// Assets whose id has none of these -- notably the per-token Base and
/// Arbitrum stablecoins, which the bridge does not index at all -- are left
/// out. They render as raw base units against their asset id, which is honest
/// about what the parser knows.
const SEEDS: &[(&str, &str, u8)] = &[
    ("nep141:wrap.near", "wNEAR", 24),
    // Legacy rainbow bridge.
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
    ("nep141:eth.bridge.near", "ETH.e", 18),
    // Omni bridge, chain-native assets. `eth`/`sol` are the two ids carried by
    // observed intent traffic.
    ("nep141:eth.omft.near", "ETH", 18),
    ("nep141:sol.omft.near", "SOL", 9),
    ("nep141:btc.omft.near", "BTC", 8),
    ("nep141:nbtc.bridge.near", "NBTC", 8),
    ("nep141:pol.omft.near", "POL", 18),
    ("nep141:base.omft.near", "ETH.base", 18),
    ("nep141:arb.omft.near", "ETH.arb", 18),
    // Omni bridge, per-token assets.
    (
        "nep141:eth-0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48.omft.near",
        "USDC",
        6,
    ),
    (
        "nep141:sol-5ce3bf3a31af18be40ba30f721101b4341690186.omft.near",
        "USDC.sol",
        6,
    ),
    (
        "nep141:sol-c800a4bd850783ccb82c2b2c7e84175443606352.omft.near",
        "USDT.sol",
        6,
    ),
    (
        "nep141:eth-0xdac17f958d2ee523a2206206994597c13d831ec7.omft.near",
        "USDT",
        6,
    ),
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

    /// The two assets carried by real intent traffic. Both were rendering as
    /// raw base units against an unresolved asset id, so nothing on screen
    /// distinguished 1 ETH from 1 gwei.
    #[test]
    fn the_omni_bridge_native_assets_resolve() {
        let eth = resolve("nep141:eth.omft.near", &empty()).expect("seeded");
        assert_eq!((eth.symbol.as_str(), eth.decimals), ("ETH", 18));
        let sol = resolve("nep141:sol.omft.near", &empty()).expect("seeded");
        assert_eq!((sol.symbol.as_str(), sol.decimals), ("SOL", 9));
    }

    /// `omni.bridge.near::get_native_token_id("Eth")` returns the legacy
    /// rainbow-bridge id, which is how the wrong entry reached the table. Both
    /// ids are real and both are ETH, so they have to stay distinguishable.
    #[test]
    fn the_legacy_and_omni_ether_ids_do_not_render_alike() {
        let legacy = resolve("nep141:eth.bridge.near", &empty()).expect("seeded");
        let omni = resolve("nep141:eth.omft.near", &empty()).expect("seeded");
        assert_ne!(legacy.symbol, omni.symbol);
    }

    /// Four distinct assets report `{"symbol":"ETH","decimals":18}` on-chain
    /// and four report `{"symbol":"USDC","decimals":6}`. `AmountV2` carries
    /// only an amount and an abbreviation, so an unqualified symbol would let
    /// Base ETH render identically to mainnet ETH -- and a swap of one for the
    /// other render identically to its mirror.
    #[test]
    fn every_seeded_symbol_is_unique() {
        let mut seen = std::collections::BTreeMap::new();
        for (asset_id, symbol, _) in SEEDS {
            if let Some(previous) = seen.insert(*symbol, *asset_id) {
                panic!("symbol {symbol} is shared by {previous} and {asset_id}");
            }
        }
    }

    /// The same token bridged from a different chain carries an origin
    /// qualifier; the asset native to its own chain carries the bare symbol.
    #[test]
    fn same_token_from_different_chains_is_origin_qualified() {
        for (asset_id, expected) in [
            ("nep141:base.omft.near", "ETH.base"),
            ("nep141:arb.omft.near", "ETH.arb"),
            (
                "nep141:eth-0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48.omft.near",
                "USDC",
            ),
        ] {
            let meta = resolve(asset_id, &empty()).expect(asset_id);
            assert_eq!(meta.symbol, expected, "for {asset_id}");
        }
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
