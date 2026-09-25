//! Verifying an NSM attestation doc and reading how long it stays valid.

use std::time::Duration;

use qos_nsm::nitro;
use x509_cert::der::Decode as _;

use crate::AttestationError;

/// How long a verified attestation doc's certificate chain stays valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CertValidity {
    /// The doc's own NSM timestamp (ms since the unix epoch).
    pub timestamp_ms: u64,
    /// Earliest `notAfter` across the leaf certificate and the CA bundle
    /// (seconds since the unix epoch). In practice this is the leaf, which NSM
    /// issues for ~3h.
    pub not_after_unix: u64,
    /// `not_after_unix` minus the doc's timestamp; zero if already expired.
    pub remaining: Duration,
}

/// Verify `document` (COSE Sign1) against the AWS Nitro root at the doc's own
/// NSM timestamp, then read the chain's earliest `notAfter`.
///
/// Validation uses the NSM timestamp rather than the enclave clock, which isn't
/// trusted; callers turn `remaining` into a deadline on a monotonic clock.
///
/// # Errors
///
/// [`AttestationError::Certificate`] if the doc doesn't decode, its signature
/// or chain doesn't verify, or a certificate doesn't parse.
pub fn cert_validity(document: &[u8]) -> Result<CertValidity, AttestationError> {
    let cert_err = |what: &str, e: &dyn std::fmt::Debug| {
        AttestationError::Certificate(format!("{what}: {e:?}"))
    };
    let root = nitro::cert_from_pem(nitro::AWS_ROOT_CERT_PEM)
        .map_err(|e| cert_err("AWS root cert", &e))?;
    let timestamp_ms = nitro::unsafe_attestation_doc_from_der(document)
        .map_err(|e| cert_err("decode", &e))?
        .timestamp;
    let doc = nitro::attestation_doc_from_der(document, &root, timestamp_ms / 1000)
        .map_err(|e| cert_err("verify", &e))?;

    let mut not_after_unix = u64::MAX;
    let chain = std::iter::once(doc.certificate.as_slice())
        .chain(doc.cabundle.iter().map(|c| c.as_slice()));
    for der in chain {
        let cert = x509_cert::Certificate::from_der(der).map_err(|e| cert_err("parse", &e))?;
        let not_after = cert
            .tbs_certificate
            .validity
            .not_after
            .to_unix_duration()
            .as_secs();
        not_after_unix = not_after_unix.min(not_after);
    }

    Ok(CertValidity {
        timestamp_ms,
        not_after_unix,
        remaining: Duration::from_secs(not_after_unix.saturating_sub(timestamp_ms / 1000)),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
pub(crate) mod tests {
    use super::*;

    /// Real Nitro attestation doc (tkhq/qos `qos_nsm/src/static/mock_attestation_doc`
    /// at 82d8e18b). Metadata:
    /// - NSM timestamp: 1657117102484 ms (2022-07-06T14:18:22.484Z)
    /// - leaf `notAfter`: 2022-07-06T17:18:22Z = 1657127902 (earliest in chain)
    /// - `cabundle[3]` (instance) `notAfter`: 2022-07-07T10:01:19Z = 1657188079
    pub(crate) const NITRO_DOC: &[u8] = include_bytes!("../tests/fixtures/nitro_attestation_doc");
    const NITRO_DOC_TIMESTAMP_MS: u64 = 1_657_117_102_484;
    const NITRO_LEAF_NOT_AFTER: u64 = 1_657_127_902;

    #[test]
    fn reads_leaf_expiry_from_real_doc() {
        let v = cert_validity(NITRO_DOC).unwrap();
        assert_eq!(v.timestamp_ms, NITRO_DOC_TIMESTAMP_MS);
        assert_eq!(
            v.not_after_unix, NITRO_LEAF_NOT_AFTER,
            "earliest notAfter is the leaf"
        );
        assert_eq!(
            v.remaining,
            Duration::from_secs(3 * 60 * 60),
            "NSM leaf lives 3h"
        );
    }

    #[test]
    fn validates_at_doc_timestamp_not_wall_clock() {
        // The 2022 chain is long expired by wall-clock time; it must still
        // verify because validation happens at the doc's own NSM timestamp.
        assert!(cert_validity(NITRO_DOC).is_ok());
    }

    #[test]
    fn rejects_tampered_signature() {
        let mut doc = NITRO_DOC.to_vec();
        let last = doc.len() - 1;
        doc[last] ^= 0x01;
        let err = cert_validity(&doc).unwrap_err();
        assert!(err.to_string().contains("verify"), "{err}");
    }

    #[test]
    fn rejects_truncated_and_empty_docs() {
        for doc in [&NITRO_DOC[..NITRO_DOC.len() / 2], &[][..]] {
            let err = cert_validity(doc).unwrap_err();
            assert!(err.to_string().contains("decode"), "{err}");
        }
    }
}
