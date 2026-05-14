//! x402 configuration loaded from env vars + named profiles.

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use std::str::FromStr;
use std::time::Duration;
use url::Url;

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

// ── Wire types for X402_PRICE_TAGS_JSON deserialization ─────────────────────

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
            (Some(s), None) => Ok(PayToAddress::Evm(s)),
            (None, Some(s)) => Ok(PayToAddress::Solana(s)),
            _ => Err(ConfigError::Invalid {
                var: "X402_PRICE_TAGS_JSON",
                message: "payTo must specify exactly one of evm or solana".into(),
            }),
        }
    }
}

// ── X402Config env loader ────────────────────────────────────────────────────

impl X402Config {
    /// Production entrypoint — reads the real process environment.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Test-friendly core — takes a closure that resolves env-var lookups.
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
        let protocol_version = get("X402_PROTOCOL_VERSION").unwrap_or_else(|| "v2".to_string());

        let price_tags = if let Some(json) = get("X402_PRICE_TAGS_JSON") {
            Self::parse_tags_json(&json)?
        } else {
            vec![Self::seeded_tag(&get, profile)?]
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
        Url::parse(&s).map_err(|e| ConfigError::Invalid {
            var: "X402_FACILITATOR_URL",
            message: e.to_string(),
        })
    }

    fn load_timeout<F>(get: &F) -> Result<Duration, ConfigError>
    where
        F: Fn(&str) -> Option<String>,
    {
        match get("X402_FACILITATOR_TIMEOUT_SECS") {
            Some(s) => {
                s.parse::<u64>()
                    .map(Duration::from_secs)
                    .map_err(|e| ConfigError::Invalid {
                        var: "X402_FACILITATOR_TIMEOUT_SECS",
                        message: e.to_string(),
                    })
            }
            None => Ok(Duration::from_secs(5)),
        }
    }

    fn seeded_tag<F>(get: &F, profile: X402Profile) -> Result<PriceTagConfig, ConfigError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let (network, price_str, default_payto): (&str, &str, Option<PayToAddress>) = match profile
        {
            X402Profile::Local => (
                "base-sepolia",
                "0.0001",
                Some(PayToAddress::Evm(
                    "0x000000000000000000000000000000000000dEaD".to_string(),
                )),
            ),
            X402Profile::PayAi => ("base", "0.001", None),
            X402Profile::Custom => {
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
        if s.starts_with("0x") && s.len() == 42 {
            Ok(PayToAddress::Evm(s.to_string()))
        } else if !s.is_empty() && !s.starts_with("0x") {
            Ok(PayToAddress::Solana(s.to_string()))
        } else {
            Err(ConfigError::Invalid {
                var: "X402_PAYTO",
                message: "not a recognizable EVM or Solana address".into(),
            })
        }
    }

    fn parse_tags_json(json: &str) -> Result<Vec<PriceTagConfig>, ConfigError> {
        let wire: Vec<PriceTagWire> =
            serde_json::from_str(json).map_err(|e| ConfigError::JsonParse(e.to_string()))?;
        wire.into_iter()
            .map(|w| {
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

// ── X402Middleware builder ────────────────────────────────────────────────────

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
        let m = x402_axum::X402Middleware::try_new(self.facilitator_url.as_str()).map_err(|e| {
            ConfigError::Invalid {
                var: "X402_FACILITATOR_URL",
                message: e.to_string(),
            }
        })?;

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

/// Convert a single [`PriceTagConfig`] into a [`v2::PriceTag`].
fn build_price_tag(tag: &PriceTagConfig) -> Result<v2::PriceTag, ConfigError> {
    if tag.scheme != PriceScheme::Exact {
        return Err(ConfigError::Invalid {
            var: "X402_PRICE_TAGS_JSON",
            message: "unsupported scheme; only 'exact' is supported".into(),
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

    match (&tag.pay_to, tag.network.as_str()) {
        (PayToAddress::Evm(addr_s), "base-sepolia") => {
            let addr: ChecksummedAddress =
                addr_s
                    .parse()
                    .map_err(
                        |e: <ChecksummedAddress as FromStr>::Err| ConfigError::Invalid {
                            var: "payTo.evm",
                            message: format!("invalid EVM address '{addr_s}': {e}"),
                        },
                    )?;
            Ok(V2Eip155Exact::price_tag(
                addr,
                USDC::base_sepolia().amount(atomic),
            ))
        }
        (PayToAddress::Evm(addr_s), "base") => {
            let addr: ChecksummedAddress =
                addr_s
                    .parse()
                    .map_err(
                        |e: <ChecksummedAddress as FromStr>::Err| ConfigError::Invalid {
                            var: "payTo.evm",
                            message: format!("invalid EVM address '{addr_s}': {e}"),
                        },
                    )?;
            Ok(V2Eip155Exact::price_tag(addr, USDC::base().amount(atomic)))
        }
        (PayToAddress::Solana(addr_s), "solana") => {
            let addr: SolanaAddress =
                addr_s
                    .parse()
                    .map_err(|e: <SolanaAddress as FromStr>::Err| ConfigError::Invalid {
                        var: "payTo.solana",
                        message: format!("invalid Solana address '{addr_s}': {e}"),
                    })?;
            Ok(V2SolanaExact::price_tag(
                addr,
                USDC::solana().amount(atomic),
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
        assert_eq!(cfg.price_tags.len(), 1);
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
    fn from_env_malformed_tags_json_rejected() {
        let err =
            X402Config::from_lookup(lookup(&[("X402_PRICE_TAGS_JSON", "not json")])).unwrap_err();
        assert!(matches!(err, ConfigError::JsonParse(_)));
    }

    #[test]
    fn from_env_rejects_unsupported_scheme() {
        let json = r#"[
            {"network":"base","asset":"USDC","priceUsd":"0.05","payTo":{"evm":"0x1111111111111111111111111111111111111111"},"scheme":"upto"}
        ]"#;
        let err = X402Config::from_lookup(lookup(&[("X402_PRICE_TAGS_JSON", json)])).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid { .. }));
    }
}
