//! x402 configuration loaded from env vars + named profiles.

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use std::str::FromStr;
use std::time::Duration;
use url::Url;
use visualsign::encodings::split_hex_prefix;

/// Default Solana "burn"-style payTo used in the local profile when an
/// operator hasn't supplied an explicit one. Solana System Program ID is
/// 32 zero bytes; sending USDC there sinks it without crediting anyone.
const SOLANA_BURN_PAYTO: &str = "11111111111111111111111111111111";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum X402Profile {
    Local,
    PayAi,
    Custom,
}

impl FromStr for X402Profile {
    type Err = ConfigError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "local" => Ok(X402Profile::Local),
            "payai" => Ok(X402Profile::PayAi),
            "custom" => Ok(X402Profile::Custom),
            other => Err(ConfigError::UnknownProfile(other.to_string())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PayToAddress {
    Evm(String),    // 0x-prefixed 20-byte hex
    Solana(String), // base58 32-byte pubkey
}

#[derive(Debug, Clone, PartialEq)]
pub struct PriceTagConfig {
    pub network: String, // e.g. "base-sepolia", "base", "solana"
    pub asset: String,   // e.g. "USDC"
    pub price_usd: Decimal,
    pub pay_to: PayToAddress,
    pub scheme: PriceScheme, // currently only "exact" is supported for v2 tags
}

#[derive(Debug, Clone)]
pub struct X402Config {
    pub profile: X402Profile,
    pub facilitator_url: Url,
    pub facilitator_timeout: Duration,
    pub protocol_version: String, // "v2"
    pub price_tags: Vec<PriceTagConfig>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("unknown X402_PROFILE: {0}")]
    UnknownProfile(String),
    #[error("missing required env var: {0}")]
    MissingVar(&'static str),
    #[error("invalid env var {var}: {message}")]
    Invalid { var: &'static str, message: String },
    #[error("X402_PRICE_TAGS_JSON parse error: {0}")]
    JsonParse(String),
}

// -- Wire types for X402_PRICE_TAGS_JSON deserialization ---------------------

use serde::Deserialize;

#[derive(Deserialize)]
struct PayToWire {
    evm: Option<String>,
    solana: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PriceTagWire {
    network: String,
    asset: String,
    price_usd: String,
    pay_to: PayToWire,
    #[serde(default = "default_scheme")]
    scheme: String,
}

fn default_scheme() -> String {
    "exact".to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriceScheme {
    Exact,
}

impl FromStr for PriceScheme {
    type Err = ConfigError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "exact" => Ok(Self::Exact),
            other => Err(ConfigError::Invalid {
                var: "X402_PRICE_TAGS_JSON",
                message: format!("unsupported scheme '{other}'; only 'exact' is supported"),
            }),
        }
    }
}

impl PayToWire {
    fn into_pay_to(self) -> Result<PayToAddress, ConfigError> {
        match (self.evm, self.solana) {
            // Rebuild with a canonical lowercase `0x` prefix, same as
            // `classify_payto` does for the `X402_PAYTO` env var path:
            // downstream `ChecksummedAddress::from_str` only strips
            // lowercase `0x`, so a valid uppercase-prefixed `0X` address
            // here would otherwise reach it unchanged and be rejected.
            (Some(s), None) => {
                let normalized = match split_hex_prefix(&s) {
                    Some(hex_body) => format!("0x{hex_body}"),
                    None => s,
                };
                Ok(PayToAddress::Evm(normalized))
            }
            (None, Some(s)) => Ok(PayToAddress::Solana(s)),
            _ => Err(ConfigError::Invalid {
                var: "X402_PRICE_TAGS_JSON",
                message: "payTo must specify exactly one of evm or solana".into(),
            }),
        }
    }
}

// -- X402Config env loader ----------------------------------------------------

/// Every env var `from_lookup` (or a function it calls) reads by name.
const X402_ENV_KEYS: &[&str] = &[
    "X402_PROFILE",
    "X402_FACILITATOR_URL",
    "X402_FACILITATOR_TIMEOUT_SECS",
    "X402_PROTOCOL_VERSION",
    "X402_PRICE_TAGS_JSON",
    "X402_NETWORK",
    "X402_PAYTO",
];

impl X402Config {
    /// Production entrypoint -- reads the real process environment.
    ///
    /// Distinguishes "unset" from "set but not valid UTF-8" for every x402
    /// env var (see `crate::env_util::checked_env_var`): plain
    /// `std::env::var(..).ok()` collapses both into `None`, which would
    /// silently fall back to seeded defaults (or the profile default) for a
    /// malformed value instead of reporting invalid configuration.
    pub fn from_env() -> Result<Self, ConfigError> {
        let mut resolved = std::collections::BTreeMap::new();
        for key in X402_ENV_KEYS {
            resolved.insert(*key, Self::checked_env_var(key)?);
        }
        Self::from_lookup(|key| resolved.get(key).cloned().flatten())
    }

    /// Reads an env var, distinguishing "unset" from "set but not valid
    /// UTF-8". See `crate::env_util::checked_env_var`.
    fn checked_env_var(key: &'static str) -> Result<Option<String>, ConfigError> {
        crate::env_util::checked_env_var(key, |var| ConfigError::Invalid {
            var,
            message: "contains invalid (non-UTF-8) bytes".to_string(),
        })
    }

    /// Test-friendly core -- takes a closure that resolves env-var lookups.
    /// All env reads in the loader go through this closure, so tests can
    /// inject fixed values without mutating process state.
    pub(crate) fn from_lookup<F>(get: F) -> Result<Self, ConfigError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let profile = get("X402_PROFILE")
            .unwrap_or_else(|| "local".to_string())
            .parse::<X402Profile>()?;

        let facilitator_url = Self::load_facilitator_url(&get, profile)?;
        let facilitator_timeout = Self::load_timeout(&get)?;
        // Only x402 protocol v2 is wired up (build_middleware hardcodes
        // x402_types::proto::v2); reject anything else at load time instead
        // of silently serving v2 under a different declared version.
        let protocol_version = get("X402_PROTOCOL_VERSION").unwrap_or_else(|| "v2".to_string());
        if protocol_version != "v2" {
            return Err(ConfigError::Invalid {
                var: "X402_PROTOCOL_VERSION",
                message: format!("unsupported value '{protocol_version}'; only 'v2' is supported"),
            });
        }

        let price_tags = if let Some(json) = get("X402_PRICE_TAGS_JSON") {
            Self::parse_tags_json(&json)?
        } else {
            let mut tags = vec![Self::seeded_tag(&get, profile)?];
            // Local profile, derive mode (no X402_NETWORK override): also
            // offer a Solana tag with a burn-style payTo so a single local
            // gateway answers both CHAIN_ETHEREUM (base-sepolia) and
            // CHAIN_SOLANA (solana-devnet) requests out of the box. Other
            // profiles stay single-chain by default; operators wanting
            // multi-chain in payai/custom use X402_PRICE_TAGS_JSON.
            if profile == X402Profile::Local && get("X402_NETWORK").is_none() {
                let price_usd = Decimal::from_str("0.0001").map_err(|e| ConfigError::Invalid {
                    var: "(internal seed price)",
                    message: e.to_string(),
                })?;
                tags.push(PriceTagConfig {
                    network: "solana-devnet".to_string(),
                    asset: "USDC".to_string(),
                    price_usd,
                    pay_to: PayToAddress::Solana(SOLANA_BURN_PAYTO.to_string()),
                    scheme: PriceScheme::Exact,
                });
            }
            tags
        };

        if price_tags.is_empty() {
            return Err(ConfigError::Invalid {
                var: "X402_PRICE_TAGS_JSON",
                message: "must contain at least one tag".into(),
            });
        }

        Ok(X402Config {
            profile,
            facilitator_url,
            facilitator_timeout,
            protocol_version,
            price_tags,
        })
    }

    fn load_facilitator_url<F>(get: &F, profile: X402Profile) -> Result<Url, ConfigError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let s = match (get("X402_FACILITATOR_URL"), profile) {
            (Some(s), _) => s,
            (None, X402Profile::Local) => "http://127.0.0.1:8090".to_string(),
            (None, X402Profile::PayAi) => "https://facilitator.payai.network".to_string(),
            (None, X402Profile::Custom) => {
                return Err(ConfigError::MissingVar("X402_FACILITATOR_URL"));
            }
        };
        let url = Url::parse(&s).map_err(|e| ConfigError::Invalid {
            var: "X402_FACILITATOR_URL",
            message: e.to_string(),
        })?;
        // x402 requests carry signed payment authorization, so plain http
        // exposes replayable payment material to an on-path attacker. Only
        // accept https, with one carve-out: `local` may use http against a
        // loopback facilitator for zero-config dev/CI, since that traffic
        // never leaves the host.
        let loopback_http_in_local = profile == X402Profile::Local
            && url.scheme() == "http"
            && matches!(
                url.host_str(),
                Some("127.0.0.1") | Some("localhost") | Some("::1")
            );
        if url.scheme() != "https" && !loopback_http_in_local {
            return Err(ConfigError::Invalid {
                var: "X402_FACILITATOR_URL",
                message: format!(
                    "scheme '{}' not allowed; use https (or http against a loopback host under X402_PROFILE=local)",
                    url.scheme()
                ),
            });
        }
        // Userinfo (user:pass@host) isn't part of this configuration
        // contract, and the URL is later logged verbatim on probe failure
        // (main.rs), which would leak credentials into gateway logs. Reject
        // it outright rather than relying on every call site to redact.
        if !url.username().is_empty() || url.password().is_some() {
            return Err(ConfigError::Invalid {
                var: "X402_FACILITATOR_URL",
                message: "must not contain userinfo (user:pass@host)".into(),
            });
        }
        Ok(url)
    }

    fn load_timeout<F>(get: &F) -> Result<Duration, ConfigError>
    where
        F: Fn(&str) -> Option<String>,
    {
        match get("X402_FACILITATOR_TIMEOUT_SECS") {
            Some(s) => {
                let secs = s.parse::<u64>().map_err(|e| ConfigError::Invalid {
                    var: "X402_FACILITATOR_TIMEOUT_SECS",
                    message: e.to_string(),
                })?;
                // 0 parses to Duration::ZERO, which times out every
                // facilitator call instantly rather than disabling the
                // timeout; reject it instead of silently taking x402 down.
                if secs == 0 {
                    return Err(ConfigError::Invalid {
                        var: "X402_FACILITATOR_TIMEOUT_SECS",
                        message: "must be greater than 0".to_string(),
                    });
                }
                Ok(Duration::from_secs(secs))
            }
            None => Ok(Duration::from_secs(5)),
        }
    }

    fn seeded_tag<F>(get: &F, profile: X402Profile) -> Result<PriceTagConfig, ConfigError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let network_override = get("X402_NETWORK");
        let (network, price_str, default_payto): (&str, &str, Option<PayToAddress>) =
            match (profile, network_override.as_deref()) {
                // Explicit override takes priority over profile defaults. The default
                // payTo only makes sense for the local burn-address case; everywhere
                // else the operator must set X402_PAYTO.
                (_, Some("base-sepolia")) => ("base-sepolia", "0.0001", None),
                (_, Some("base")) => ("base", "0.001", None),
                (_, Some("solana")) => ("solana", "0.001", None),
                (_, Some("solana-devnet")) => ("solana-devnet", "0.001", None),
                (_, Some(other)) => {
                    return Err(ConfigError::Invalid {
                        var: "X402_NETWORK",
                        message: format!(
                            "unsupported network '{other}'; expected one of \
                         base-sepolia, base, solana, solana-devnet"
                        ),
                    });
                }
                // Profile defaults when X402_NETWORK is unset.
                (X402Profile::Local, None) => (
                    "base-sepolia",
                    "0.0001",
                    Some(PayToAddress::Evm(
                        "0x000000000000000000000000000000000000dEaD".to_string(),
                    )),
                ),
                (X402Profile::PayAi, None) => ("base", "0.001", None),
                (X402Profile::Custom, None) => {
                    return Err(ConfigError::MissingVar("X402_PRICE_TAGS_JSON"));
                }
            };

        let price_usd = Decimal::from_str(price_str).map_err(|e| ConfigError::Invalid {
            var: "(internal seed price)",
            message: e.to_string(),
        })?;

        let pay_to = match (get("X402_PAYTO"), default_payto) {
            (Some(s), _) => Self::classify_payto(&s)?,
            (None, Some(p)) => p,
            (None, None) => return Err(ConfigError::MissingVar("X402_PAYTO")),
        };

        Ok(PriceTagConfig {
            network: network.to_string(),
            asset: "USDC".to_string(),
            price_usd,
            pay_to,
            scheme: PriceScheme::Exact,
        })
    }

    fn classify_payto(s: &str) -> Result<PayToAddress, ConfigError> {
        // Use the repo's unified (case-insensitive) 0x/0X prefix handling
        // rather than hand-rolling a lowercase-only check, which would
        // misclassify a valid `0X`-prefixed EVM address as Solana. Rebuild
        // with a canonical lowercase `0x` prefix: downstream
        // `ChecksummedAddress::from_str` only strips lowercase `0x` (it
        // rejects `0X` outright with `InvalidStringLength` since the
        // unstripped `X` isn't a hex digit), so a `0X` input must be
        // normalized here to actually parse later.
        match split_hex_prefix(s) {
            Some(hex_body) if hex_body.len() == 40 => {
                Ok(PayToAddress::Evm(format!("0x{hex_body}")))
            }
            None if !s.is_empty() => Ok(PayToAddress::Solana(s.to_string())),
            _ => Err(ConfigError::Invalid {
                var: "X402_PAYTO",
                message: "not a recognizable EVM or Solana address".into(),
            }),
        }
    }

    fn parse_tags_json(json: &str) -> Result<Vec<PriceTagConfig>, ConfigError> {
        let wire: Vec<PriceTagWire> =
            serde_json::from_str(json).map_err(|e| ConfigError::JsonParse(e.to_string()))?;
        wire.into_iter()
            .map(|w| {
                // build_price_tag only ever constructs a USDC price tag; an
                // operator configuring (or typo'ing) any other asset would
                // otherwise silently get a USDC tag with no error or log.
                if w.asset != "USDC" {
                    return Err(ConfigError::Invalid {
                        var: "X402_PRICE_TAGS_JSON",
                        message: format!(
                            "unsupported asset '{}'; only 'USDC' is supported",
                            w.asset
                        ),
                    });
                }
                Ok(PriceTagConfig {
                    network: w.network,
                    asset: w.asset,
                    price_usd: Decimal::from_str(&w.price_usd).map_err(|e| {
                        ConfigError::Invalid {
                            var: "X402_PRICE_TAGS_JSON",
                            message: format!("priceUsd: {e}"),
                        }
                    })?,
                    pay_to: w.pay_to.into_pay_to()?,
                    scheme: w.scheme.parse()?,
                })
            })
            .collect()
    }
}

// -- X402Middleware builder ----------------------------------------------------

use std::sync::Arc;
use x402_axum::X402LayerBuilder;
use x402_axum::facilitator_client::FacilitatorClient;
use x402_axum::paygate::StaticPriceTags;
use x402_chain_eip155::KnownNetworkEip155;
use x402_chain_eip155::V2Eip155Exact;
use x402_chain_eip155::chain::ChecksummedAddress;
use x402_chain_solana::KnownNetworkSolana;
use x402_chain_solana::V2SolanaExact;
use x402_chain_solana::chain::Address as SolanaAddress;
use x402_types::networks::USDC;
use x402_types::proto::v2;

impl X402Config {
    /// Build an `X402LayerBuilder` from the configured price tags.
    ///
    /// Returns an error if the facilitator URL is invalid, any address cannot be
    /// parsed, the price produces arithmetic overflow, or a (payTo, network)
    /// combination is unsupported.
    pub fn build_middleware(
        &self,
    ) -> Result<X402LayerBuilder<StaticPriceTags<v2::PriceTag>, Arc<FacilitatorClient>>, ConfigError>
    {
        let facilitator = FacilitatorClient::try_new(self.facilitator_url.clone())
            .map_err(|e| ConfigError::Invalid {
                var: "X402_FACILITATOR_URL",
                message: e.to_string(),
            })?
            .with_timeout(self.facilitator_timeout);
        let m = x402_axum::X402Middleware::from_facilitator(Arc::new(facilitator));

        // Convert all price tags to v2::PriceTag.
        let tags: Vec<v2::PriceTag> = self
            .price_tags
            .iter()
            .map(build_price_tag)
            .collect::<Result<Vec<_>, _>>()?;

        // At least one tag is guaranteed by from_env validation, but handle
        // the degenerate case safely rather than panicking.
        let mut iter = tags.into_iter();
        let first = iter.next().ok_or_else(|| ConfigError::Invalid {
            var: "X402_PRICE_TAGS_JSON",
            message: "must contain at least one tag".into(),
        })?;

        let mut builder = m.with_price_tag(first);
        for tag in iter {
            builder = builder.with_price_tag(tag);
        }

        Ok(builder)
    }
}

/// Parse a payTo address string into its typed chain-address representation,
/// wrapping a parse failure into the `ConfigError` shape shared by every
/// (payTo, network) match arm in `build_price_tag`.
fn parse_payto_addr<T>(addr_s: &str, kind: &str, field: &'static str) -> Result<T, ConfigError>
where
    T: FromStr,
    T::Err: std::fmt::Display,
{
    addr_s.parse().map_err(|e: T::Err| ConfigError::Invalid {
        var: field,
        message: format!("invalid {kind} address '{addr_s}': {e}"),
    })
}

/// Convert a single [`PriceTagConfig`] into a [`v2::PriceTag`].
fn build_price_tag(tag: &PriceTagConfig) -> Result<v2::PriceTag, ConfigError> {
    if tag.scheme != PriceScheme::Exact {
        return Err(ConfigError::Invalid {
            var: "X402_PRICE_TAGS_JSON",
            message: "unsupported scheme; only 'exact' is supported".into(),
        });
    }

    if tag.price_usd.is_sign_negative() {
        return Err(ConfigError::Invalid {
            var: "priceUsd",
            message: format!("price {} must not be negative", tag.price_usd),
        });
    }

    // USDC has 6 decimals on all supported networks.
    // price_usd * 1_000_000 = atomic units.
    let atomic = tag
        .price_usd
        .checked_mul(Decimal::from(1_000_000u64))
        .and_then(|d| d.round().to_u64())
        .ok_or_else(|| ConfigError::Invalid {
            var: "priceUsd",
            message: format!("price {} overflows USDC atomic units (u64)", tag.price_usd),
        })?;

    if atomic == 0 {
        return Err(ConfigError::Invalid {
            var: "priceUsd",
            message: format!(
                "price {} rounds to 0 USDC atomic units; the route would be free",
                tag.price_usd
            ),
        });
    }

    match (&tag.pay_to, tag.network.as_str()) {
        (PayToAddress::Evm(addr_s), "base-sepolia") => {
            let addr: ChecksummedAddress = parse_payto_addr(addr_s, "EVM", "payTo.evm")?;
            Ok(V2Eip155Exact::price_tag(
                addr,
                USDC::base_sepolia().amount(atomic),
            ))
        }
        (PayToAddress::Evm(addr_s), "base") => {
            let addr: ChecksummedAddress = parse_payto_addr(addr_s, "EVM", "payTo.evm")?;
            Ok(V2Eip155Exact::price_tag(addr, USDC::base().amount(atomic)))
        }
        (PayToAddress::Solana(addr_s), "solana") => {
            let addr: SolanaAddress = parse_payto_addr(addr_s, "Solana", "payTo.solana")?;
            Ok(V2SolanaExact::price_tag(
                addr,
                USDC::solana().amount(atomic),
            ))
        }
        (PayToAddress::Solana(addr_s), "solana-devnet") => {
            let addr: SolanaAddress = parse_payto_addr(addr_s, "Solana", "payTo.solana")?;
            Ok(V2SolanaExact::price_tag(
                addr,
                USDC::solana_devnet().amount(atomic),
            ))
        }
        (pay_to, network) => Err(ConfigError::Invalid {
            var: "X402_PRICE_TAGS_JSON",
            message: format!(
                "unsupported (payTo, network) combination: ({:?}, {network:?})",
                match pay_to {
                    PayToAddress::Evm(_) => "evm",
                    PayToAddress::Solana(_) => "solana",
                }
            ),
        }),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn profile_parses_local() {
        assert_eq!("local".parse::<X402Profile>().unwrap(), X402Profile::Local);
    }

    #[test]
    fn profile_parses_payai() {
        assert_eq!("payai".parse::<X402Profile>().unwrap(), X402Profile::PayAi);
    }

    #[test]
    fn profile_parses_custom() {
        assert_eq!(
            "custom".parse::<X402Profile>().unwrap(),
            X402Profile::Custom
        );
    }

    #[test]
    fn profile_rejects_unknown() {
        assert!("nope".parse::<X402Profile>().is_err());
    }

    // --- env-loader tests (no env mutation; pure closure-driven) ---

    fn lookup<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |key| {
            pairs.iter().find_map(|(k, v)| {
                if *k == key {
                    Some((*v).to_string())
                } else {
                    None
                }
            })
        }
    }

    #[test]
    fn from_env_local_defaults() {
        let cfg = X402Config::from_lookup(lookup(&[])).unwrap();
        assert_eq!(cfg.profile, X402Profile::Local);
        assert_eq!(cfg.facilitator_url.as_str(), "http://127.0.0.1:8090/");
        assert_eq!(cfg.facilitator_timeout, Duration::from_secs(5));
        assert_eq!(cfg.protocol_version, "v2");
        // Local profile + derive mode emits BOTH chains so a single gateway
        // can answer CHAIN_ETHEREUM and CHAIN_SOLANA requests with no extra
        // config. EVM tag first (legacy ordering), Solana tag appended.
        assert_eq!(cfg.price_tags.len(), 2);
        assert_eq!(cfg.price_tags[0].network, "base-sepolia");
        assert_eq!(cfg.price_tags[0].asset, "USDC");
        assert_eq!(
            cfg.price_tags[0].price_usd,
            Decimal::from_str("0.0001").unwrap()
        );
        assert_eq!(
            cfg.price_tags[0].pay_to,
            PayToAddress::Evm("0x000000000000000000000000000000000000dEaD".to_string())
        );
        assert_eq!(cfg.price_tags[0].scheme, PriceScheme::Exact);
        assert_eq!(cfg.price_tags[1].network, "solana-devnet");
        assert_eq!(cfg.price_tags[1].asset, "USDC");
        assert_eq!(
            cfg.price_tags[1].pay_to,
            PayToAddress::Solana(SOLANA_BURN_PAYTO.to_string())
        );
    }

    #[test]
    fn from_env_local_with_explicit_network_stays_single_chain() {
        // X402_NETWORK override -> legacy single-tag mode, no Solana auto-add.
        // (The explicit-network path strips the burn-address default, so the
        // test must supply X402_PAYTO too -- same as production callers.)
        let cfg = X402Config::from_lookup(lookup(&[
            ("X402_NETWORK", "base-sepolia"),
            ("X402_PAYTO", "0xabcdef0000000000000000000000000000000001"),
        ]))
        .unwrap();
        assert_eq!(cfg.price_tags.len(), 1);
        assert_eq!(cfg.price_tags[0].network, "base-sepolia");
    }

    #[test]
    fn from_env_payai_with_uppercase_hex_prefix_payto_classifies_as_evm() {
        // A valid `0X`-prefixed EVM address must classify as EVM, not
        // Solana, and the resulting PayToAddress must actually parse as a
        // ChecksummedAddress later in build_price_tag (0X isn't stripped by
        // the downstream hex decoder, so classify_payto must normalize it).
        let cfg = X402Config::from_lookup(lookup(&[
            ("X402_PROFILE", "payai"),
            ("X402_PAYTO", "0Xabcdef0000000000000000000000000000000001"),
        ]))
        .unwrap();
        assert_eq!(
            cfg.price_tags[0].pay_to,
            PayToAddress::Evm("0xabcdef0000000000000000000000000000000001".to_string())
        );
        let _ = build_price_tag(&cfg.price_tags[0]).expect("0X-prefixed payTo must build");
    }

    #[test]
    fn from_env_payai_requires_payto() {
        let err = X402Config::from_lookup(lookup(&[("X402_PROFILE", "payai")])).unwrap_err();
        assert!(matches!(err, ConfigError::MissingVar("X402_PAYTO")));
    }

    #[test]
    fn from_env_payai_with_payto() {
        let cfg = X402Config::from_lookup(lookup(&[
            ("X402_PROFILE", "payai"),
            ("X402_PAYTO", "0xabcdef0000000000000000000000000000000001"),
        ]))
        .unwrap();
        assert_eq!(cfg.profile, X402Profile::PayAi);
        assert_eq!(
            cfg.facilitator_url.as_str(),
            "https://facilitator.payai.network/"
        );
        assert_eq!(cfg.price_tags[0].network, "base");
        assert_eq!(
            cfg.price_tags[0].price_usd,
            Decimal::from_str("0.001").unwrap()
        );
        assert_eq!(
            cfg.price_tags[0].pay_to,
            PayToAddress::Evm("0xabcdef0000000000000000000000000000000001".to_string())
        );
    }

    #[test]
    fn from_env_custom_requires_facilitator_url() {
        let err = X402Config::from_lookup(lookup(&[("X402_PROFILE", "custom")])).unwrap_err();
        assert!(matches!(
            err,
            ConfigError::MissingVar("X402_FACILITATOR_URL")
        ));
    }

    #[test]
    fn from_env_tags_json_overrides_seed() {
        let json = r#"[
            {"network":"base","asset":"USDC","priceUsd":"0.05","payTo":{"evm":"0x1111111111111111111111111111111111111111"},"scheme":"exact"},
            {"network":"solana","asset":"USDC","priceUsd":"0.05","payTo":{"solana":"EGBQqKn968sVv5cQh5Cr72pSTHfxsuzq7o7asqYB5uEV"},"scheme":"exact"}
        ]"#;
        let cfg = X402Config::from_lookup(lookup(&[("X402_PRICE_TAGS_JSON", json)])).unwrap();
        assert_eq!(cfg.price_tags.len(), 2);
        assert_eq!(cfg.price_tags[0].network, "base");
        assert_eq!(
            cfg.price_tags[0].price_usd,
            Decimal::from_str("0.05").unwrap()
        );
        assert_eq!(cfg.price_tags[1].network, "solana");
        assert!(matches!(cfg.price_tags[1].pay_to, PayToAddress::Solana(_)));
    }

    #[test]
    fn from_env_tags_json_uppercase_hex_prefix_evm_payto_normalizes() {
        // Same normalization as the X402_PAYTO env-var path
        // (from_env_payai_with_uppercase_hex_prefix_payto_classifies_as_evm):
        // an uppercase `0X`-prefixed payTo.evm in the JSON must be rebuilt
        // with a canonical lowercase `0x` prefix, or ChecksummedAddress
        // parsing in build_price_tag would reject it later.
        let json = r#"[
            {"network":"base","asset":"USDC","priceUsd":"0.05","payTo":{"evm":"0Xabcdef0000000000000000000000000000000001"},"scheme":"exact"}
        ]"#;
        let cfg = X402Config::from_lookup(lookup(&[("X402_PRICE_TAGS_JSON", json)])).unwrap();
        assert_eq!(
            cfg.price_tags[0].pay_to,
            PayToAddress::Evm("0xabcdef0000000000000000000000000000000001".to_string())
        );
        let _ = build_price_tag(&cfg.price_tags[0]).expect("0X-prefixed JSON payTo must build");
    }

    #[test]
    fn from_env_malformed_tags_json_rejected() {
        let err =
            X402Config::from_lookup(lookup(&[("X402_PRICE_TAGS_JSON", "not json")])).unwrap_err();
        assert!(matches!(err, ConfigError::JsonParse(_)));
    }

    #[test]
    fn from_env_payai_solana_devnet_with_payto() {
        let cfg = X402Config::from_lookup(lookup(&[
            ("X402_PROFILE", "payai"),
            ("X402_NETWORK", "solana-devnet"),
            ("X402_PAYTO", "EGBQqKn968sVv5cQh5Cr72pSTHfxsuzq7o7asqYB5uEV"),
        ]))
        .unwrap();
        assert_eq!(cfg.profile, X402Profile::PayAi);
        assert_eq!(cfg.price_tags[0].network, "solana-devnet");
        assert!(matches!(cfg.price_tags[0].pay_to, PayToAddress::Solana(_)));
    }

    #[test]
    fn from_env_solana_devnet_rejects_evm_payto() {
        let err = X402Config::from_lookup(lookup(&[
            ("X402_PROFILE", "payai"),
            ("X402_NETWORK", "solana-devnet"),
            ("X402_PAYTO", "0xabcdef0000000000000000000000000000000001"),
        ]))
        .unwrap();
        // The config layer accepts the seed; build_price_tag rejects the combo.
        let err = build_price_tag(&err.price_tags[0]).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid { .. }));
    }

    #[test]
    fn from_env_unknown_network_rejected() {
        let err = X402Config::from_lookup(lookup(&[
            ("X402_PROFILE", "payai"),
            ("X402_NETWORK", "fake-net"),
            ("X402_PAYTO", "EGBQqKn968sVv5cQh5Cr72pSTHfxsuzq7o7asqYB5uEV"),
        ]))
        .unwrap_err();
        assert!(matches!(
            err,
            ConfigError::Invalid {
                var: "X402_NETWORK",
                ..
            }
        ));
    }

    #[test]
    fn build_price_tag_solana_devnet_ok() {
        let tag = PriceTagConfig {
            network: "solana-devnet".to_string(),
            asset: "USDC".to_string(),
            price_usd: Decimal::from_str("0.001").unwrap(),
            pay_to: PayToAddress::Solana(
                "EGBQqKn968sVv5cQh5Cr72pSTHfxsuzq7o7asqYB5uEV".to_string(),
            ),
            scheme: PriceScheme::Exact,
        };
        let _ = build_price_tag(&tag).expect("devnet tag must build");
    }

    #[test]
    fn from_env_rejects_unsupported_scheme() {
        let json = r#"[
            {"network":"base","asset":"USDC","priceUsd":"0.05","payTo":{"evm":"0x1111111111111111111111111111111111111111"},"scheme":"upto"}
        ]"#;
        let err = X402Config::from_lookup(lookup(&[("X402_PRICE_TAGS_JSON", json)])).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid { .. }));
    }

    #[test]
    fn from_env_rejects_non_usdc_asset() {
        let json = r#"[
            {"network":"base","asset":"ETH","priceUsd":"0.05","payTo":{"evm":"0x1111111111111111111111111111111111111111"},"scheme":"exact"}
        ]"#;
        let err = X402Config::from_lookup(lookup(&[("X402_PRICE_TAGS_JSON", json)])).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid { .. }));
    }

    #[test]
    fn from_env_rejects_non_https_facilitator_for_non_local_profile() {
        let err = X402Config::from_lookup(lookup(&[
            ("X402_PROFILE", "payai"),
            ("X402_FACILITATOR_URL", "http://facilitator.payai.network"),
            ("X402_PAYTO", "0xabcdef0000000000000000000000000000000001"),
        ]))
        .unwrap_err();
        assert!(matches!(
            err,
            ConfigError::Invalid {
                var: "X402_FACILITATOR_URL",
                ..
            }
        ));
    }

    #[test]
    fn from_env_rejects_facilitator_url_with_userinfo() {
        let err = X402Config::from_lookup(lookup(&[
            ("X402_PROFILE", "payai"),
            (
                "X402_FACILITATOR_URL",
                "https://user:secret@facilitator.payai.network",
            ),
            ("X402_PAYTO", "0xabcdef0000000000000000000000000000000001"),
        ]))
        .unwrap_err();
        assert!(matches!(
            err,
            ConfigError::Invalid {
                var: "X402_FACILITATOR_URL",
                ..
            }
        ));
    }

    #[test]
    fn from_env_rejects_zero_facilitator_timeout() {
        let err =
            X402Config::from_lookup(lookup(&[("X402_FACILITATOR_TIMEOUT_SECS", "0")])).unwrap_err();
        assert!(matches!(
            err,
            ConfigError::Invalid {
                var: "X402_FACILITATOR_TIMEOUT_SECS",
                ..
            }
        ));
    }

    #[test]
    fn from_env_rejects_unsupported_protocol_version() {
        let err = X402Config::from_lookup(lookup(&[("X402_PROTOCOL_VERSION", "v1")])).unwrap_err();
        assert!(matches!(
            err,
            ConfigError::Invalid {
                var: "X402_PROTOCOL_VERSION",
                ..
            }
        ));
    }

    #[test]
    fn from_env_accepts_explicit_v2_protocol_version() {
        let cfg = X402Config::from_lookup(lookup(&[("X402_PROTOCOL_VERSION", "v2")])).unwrap();
        assert_eq!(cfg.protocol_version, "v2");
    }

    #[test]
    fn build_price_tag_rejects_zero_atomic_price() {
        let tag = PriceTagConfig {
            network: "solana-devnet".to_string(),
            asset: "USDC".to_string(),
            price_usd: Decimal::from_str("0.0000004").unwrap(),
            pay_to: PayToAddress::Solana(
                "EGBQqKn968sVv5cQh5Cr72pSTHfxsuzq7o7asqYB5uEV".to_string(),
            ),
            scheme: PriceScheme::Exact,
        };
        let err = build_price_tag(&tag).unwrap_err();
        assert!(matches!(
            err,
            ConfigError::Invalid {
                var: "priceUsd",
                ..
            }
        ));
    }

    #[test]
    fn solana_devnet_default_prices_pinned_per_seed_path() {
        // Local-profile auto-added companion tag: 0.0001, same as the
        // local-profile base-sepolia default it's paired with.
        let derived = X402Config::from_lookup(lookup(&[])).unwrap();
        let companion = &derived.price_tags[1];
        assert_eq!(companion.network, "solana-devnet");
        assert_eq!(companion.price_usd, Decimal::from_str("0.0001").unwrap());

        // Explicit X402_NETWORK=solana-devnet override path: 0.001, same as
        // the other mainnet-adjacent explicit overrides (base, solana).
        let explicit = X402Config::from_lookup(lookup(&[
            ("X402_PROFILE", "payai"),
            ("X402_NETWORK", "solana-devnet"),
            ("X402_PAYTO", "EGBQqKn968sVv5cQh5Cr72pSTHfxsuzq7o7asqYB5uEV"),
        ]))
        .unwrap();
        assert_eq!(
            explicit.price_tags[0].price_usd,
            Decimal::from_str("0.001").unwrap()
        );
    }
}
