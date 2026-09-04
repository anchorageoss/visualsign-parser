//! Deploy-time parser configuration.
//!
//! Everything in here is fixed when the process starts and cannot be influenced by
//! a request. For the enclave binary the values come from the cmdline. The intended
//! end state is for those values to land verbatim in the `pivotArgs` of the TVC
//! deployment manifest the operators sign, so a signer can check the posture a
//! deployment runs against out of band rather than trusting a per-request signal (or
//! a log line the TEE never surfaces); wiring `tools/tvc-deploy` to emit them has not
//! landed yet (see CLAUDE.md).

use visualsign::signing::MetadataTrustPolicy;

use crate::payment_verify::PaymentPolicy;

/// Configuration a parser process is deployed with.
#[derive(Debug, Clone)]
pub struct ParserConfig {
    /// Trust posture for caller-supplied ABI mappings (Ethereum `abi_mappings`).
    pub abi_trust: MetadataTrustPolicy,
    /// Whether a gateway-signed `VerifiedPaymentMarker` is required on every
    /// `parse()` call. See [`PaymentPolicy`].
    pub payment: PaymentPolicy,
}

impl ParserConfig {
    /// Builds a config from an explicit ABI trust posture and payment policy.
    #[must_use]
    pub fn new(abi_trust: MetadataTrustPolicy, payment: PaymentPolicy) -> Self {
        Self { abi_trust, payment }
    }

    /// The permissive posture, for callers that don't parse `--accept-unsigned-abis`
    /// / `--accept-signatures-from-pubkey` cmdline flags themselves (e.g. the
    /// non-attested dev gRPC server) but still need a `ParserConfig`. Unlike
    /// [`Self::abi_trust_from_options`], this can't fail. Payment verification is
    /// disabled: no binary wires up `PaymentPolicy::Required` yet.
    #[must_use]
    pub fn accept_unsigned() -> Self {
        Self::new(MetadataTrustPolicy::AcceptUnsigned, PaymentPolicy::Disabled)
    }

    /// Resolve the two mutually exclusive ABI-trust cmdline options into a posture.
    ///
    /// `accept_unsigned` is the `--accept-unsigned-abis` flag; `signer_pubkeys` holds
    /// every `--accept-signatures-from-pubkey <hex>` value. Exactly one of the two
    /// must be supplied: passing both is contradictory, and passing neither would
    /// leave the posture implicit, which is the whole thing this replaces.
    ///
    /// # Errors
    /// Returns `Err` if both or neither option was supplied, if a supplied public key
    /// is not a valid secp256k1 key, or if `--accept-signatures-from-pubkey` was used
    /// in a build without the `ethereum` feature (nothing would consume it).
    pub fn abi_trust_from_options(
        accept_unsigned: bool,
        signer_pubkeys: &[String],
    ) -> Result<MetadataTrustPolicy, String> {
        match (accept_unsigned, signer_pubkeys.is_empty()) {
            (true, true) => Ok(MetadataTrustPolicy::AcceptUnsigned),
            (false, false) => resolve_signers(signer_pubkeys),
            (true, false) => Err(
                "--accept-unsigned-abis and --accept-signatures-from-pubkey are mutually \
                 exclusive: pick the one posture this deployment runs"
                    .to_string(),
            ),
            (false, true) => Err(
                "one of --accept-unsigned-abis or --accept-signatures-from-pubkey <hex> is \
                 required: the ABI trust posture must be chosen at deploy time so a signer \
                 can verify it"
                    .to_string(),
            ),
        }
    }
}

/// Decode deploy-time signer public keys. Lives behind the `ethereum` feature
/// because the secp256k1 decoding (and the only consumer of the allowlist) is in the
/// Ethereum chain crate.
#[cfg(feature = "ethereum")]
fn resolve_signers(signer_pubkeys: &[String]) -> Result<MetadataTrustPolicy, String> {
    visualsign_ethereum::abi_metadata::signer_allowlist_from_hex(signer_pubkeys)
        .map(MetadataTrustPolicy::RequireAllowlistedSigner)
}

/// See the `ethereum`-enabled variant above. Without that feature no ABI mappings
/// are decoded at all, so accepting the flag would advertise an enforcement that
/// does not exist.
#[cfg(not(feature = "ethereum"))]
fn resolve_signers(_signer_pubkeys: &[String]) -> Result<MetadataTrustPolicy, String> {
    Err("--accept-signatures-from-pubkey requires a build with the ethereum feature".to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A real uncompressed secp256k1 public key: the point for scalar
    /// `[0x42; 32]`, the deterministic seed the Ethereum crate's ABI-signing tests
    /// use. It has to be a valid curve point, since the require-signed path decodes
    /// it for real.
    const TEST_PUBKEY: &str = "0424653eac434488002cc06bbfb7f10fe18991e35f9fe4302dbea6d2353dc0ab1c119fc5009a032aa9fe47f5e149bb8442f71f884ccb516590686d8ff6ab91c613";

    #[test]
    fn neither_option_is_an_error() {
        let err = ParserConfig::abi_trust_from_options(false, &[])
            .expect_err("the posture must be chosen explicitly");
        assert!(err.contains("is required"), "error: {err}");
    }

    #[test]
    fn both_options_are_an_error() {
        let err = ParserConfig::abi_trust_from_options(true, &[TEST_PUBKEY.to_string()])
            .expect_err("the two postures are mutually exclusive");
        assert!(err.contains("mutually exclusive"), "error: {err}");
    }

    #[test]
    fn accept_unsigned_flag_yields_accept_unsigned_posture() {
        let policy = ParserConfig::abi_trust_from_options(true, &[]).expect("valid options");
        assert!(policy.accepts_unsigned());
    }

    #[cfg(feature = "ethereum")]
    #[test]
    fn invalid_pubkey_is_an_error() {
        let err = ParserConfig::abi_trust_from_options(false, &["nonsense".to_string()])
            .expect_err("a bad key must stop startup");
        assert!(err.contains("invalid secp256k1 public key"), "error: {err}");
    }

    /// Sanity: the require-signed path produces a posture that rejects unsigned
    /// metadata and enforces an allowlist of exactly the keys that were passed.
    #[cfg(feature = "ethereum")]
    #[test]
    fn signer_pubkey_yields_require_signed_posture() {
        let policy = ParserConfig::abi_trust_from_options(false, &[TEST_PUBKEY.to_string()])
            .expect("valid options");
        assert!(!policy.accepts_unsigned());
        assert_eq!(
            policy
                .signer_allowlist()
                .expect("require-signed enforces an allowlist")
                .len(),
            1
        );
    }

    /// The same key in compressed form and with a `0x` prefix collapses to the same
    /// single allowlist entry, so an operator's choice of encoding cannot
    /// accidentally widen the allowlist.
    #[cfg(feature = "ethereum")]
    #[test]
    fn compressed_and_uncompressed_pubkeys_collapse() {
        const COMPRESSED: &str =
            "0x0324653eac434488002cc06bbfb7f10fe18991e35f9fe4302dbea6d2353dc0ab1c";
        let policy = ParserConfig::abi_trust_from_options(
            false,
            &[TEST_PUBKEY.to_string(), COMPRESSED.to_string()],
        )
        .expect("valid options");
        assert_eq!(
            policy
                .signer_allowlist()
                .expect("require-signed enforces an allowlist")
                .len(),
            1
        );
    }

    /// A build without the `ethereum` feature decodes no ABI mappings at all, so the
    /// require-signed flag must be rejected rather than advertise enforcement that
    /// nothing performs.
    #[cfg(not(feature = "ethereum"))]
    #[test]
    fn require_signed_needs_the_ethereum_feature() {
        let err = ParserConfig::abi_trust_from_options(false, &[TEST_PUBKEY.to_string()])
            .expect_err("no ethereum feature means no ABI enforcement");
        assert!(err.contains("ethereum feature"), "error: {err}");
    }
}
