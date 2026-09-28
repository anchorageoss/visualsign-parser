//! Verifying an NSM attestation doc and reading how long it stays valid.

use std::time::Duration;

use aws_nitro_enclaves_nsm_api::api::AttestationDoc;
use qos_nsm::nitro;
use x509_cert::der::Decode as _;

use crate::AttestationError;

/// How long a verified attestation doc's certificate chain stays valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CertValidity {
    /// The doc's own NSM timestamp (ms since the unix epoch).
    pub timestamp_ms: u64,
    /// Earliest `notAfter` across the leaf certificate and the CA bundle
    /// (seconds since the unix epoch). Usually the ~3h leaf, which NSM reuses
    /// across attestations until shortly before it expires.
    pub not_after_unix: u64,
    /// `not_after_unix` minus the doc's timestamp rounded up to the second
    /// (so it never overstates); zero if already expired.
    pub remaining: Duration,
}

/// A doc whose signature and chain verified, and how long it stays valid.
#[derive(Debug, Clone)]
pub(crate) struct VerifiedDoc {
    pub cert: CertValidity,
    pub doc: AttestationDoc,
}

/// Verify `document` (COSE Sign1) against the AWS Nitro root at the doc's own
/// NSM timestamp, then read the chain's earliest `notAfter`.
///
/// This proves the doc was valid when NSM produced it, not that it is fresh:
/// it's meant for a doc just returned by the local NSM, and is crate-private
/// so it isn't mistaken for a general verifier of docs from elsewhere.
/// Validation uses the NSM timestamp rather than the enclave clock, which isn't
/// trusted; callers turn `remaining` into a deadline on a monotonic clock.
///
/// # Errors
///
/// [`AttestationError::Certificate`] if the doc doesn't decode, its signature
/// or chain doesn't verify, or a certificate doesn't parse.
pub(crate) fn verify_document(document: &[u8]) -> Result<VerifiedDoc, AttestationError> {
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

    Ok(VerifiedDoc {
        cert: CertValidity {
            timestamp_ms,
            not_after_unix,
            remaining: Duration::from_secs(
                not_after_unix.saturating_sub(timestamp_ms.div_ceil(1000)),
            ),
        },
        doc,
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

    /// A live attestation doc from `nsm_probe` (a pivot over this crate) on a
    /// non-debug TVC deployment, QOS 0.12.1, 2026-09-28. Public: the same
    /// bytes any caller of that app gets as its boot proof.
    pub(crate) const TVC_LIVE_DOC: &[u8] =
        include_bytes!("../tests/fixtures/tvc_live_attestation_doc");
    /// Its ephemeral public key, manifest hash (`user_data`) and PCR0-3; PCR0/1
    /// are the published QOS 0.12.1 values.
    pub(crate) const TVC_EPHEMERAL_KEY: &str = "043457c712fd14a4893d99138264414363b02c9cdd3015ee194c8ca726ea425960f11f4b02be483b42e6baf7d8adad0f37562db2dddda0b28e8281fe32d6088ec2049c8cd488b5c24219f7c94ad90bae452a8a6bd04b0ce4b798f30a303e86618f60f533552e7365aa99d6395dd4c23a3e12944990142aa81b0fefee3ebce5a5542d";
    pub(crate) const TVC_MANIFEST_HASH: &str =
        "5e32441cebbb831c829d82a0f31c78df0f1620d249c49734f8e8eb03bff978b2";
    pub(crate) const TVC_PCRS: [&str; 4] = [
        "a16c37b49023089be139b667cf87a32c796a9ccacacf9f0c87bf1a2f58546f2b46218ee6c0a17885f8a7d1b8cccff078",
        "a16c37b49023089be139b667cf87a32c796a9ccacacf9f0c87bf1a2f58546f2b46218ee6c0a17885f8a7d1b8cccff078",
        "21b9efbc184807662e966d34f390821309eeac6802309798826296bf3e8bec7c10edb30948c90ba67310f7b964fc500a",
        "321c3cd57bd9dc5549f349c315b93167fca1adbaf19fbb9c548101bae757970fe269e530ba684826e2f5fb043319a20f",
    ];

    #[test]
    fn reads_leaf_expiry_from_real_doc() {
        let v = verify_document(NITRO_DOC).unwrap().cert;
        assert_eq!(v.timestamp_ms, NITRO_DOC_TIMESTAMP_MS);
        assert_eq!(
            v.not_after_unix, NITRO_LEAF_NOT_AFTER,
            "earliest notAfter is the leaf"
        );
        // The leaf was issued at this doc's timestamp (to the second) for 3h;
        // the .484s is rounded up so `remaining` never overstates.
        assert_eq!(v.remaining, Duration::from_secs(3 * 60 * 60 - 1));
    }

    #[test]
    fn exposes_attested_fields() {
        let doc = verify_document(NITRO_DOC).unwrap().doc;
        assert_eq!(doc.public_key.map(|k| k.len()), Some(800));
        assert_eq!(doc.user_data.map(|d| d.len()), Some(32));
        assert_eq!(doc.nonce, None);
    }

    #[test]
    fn validates_at_doc_timestamp_not_wall_clock() {
        // The 2022 chain is long expired by wall-clock time; it must still
        // verify because validation happens at the doc's own NSM timestamp.
        assert!(verify_document(NITRO_DOC).is_ok());
    }

    #[test]
    fn rejects_tampered_signature() {
        let mut doc = NITRO_DOC.to_vec();
        let last = doc.len() - 1;
        doc[last] ^= 0x01;
        let err = verify_document(&doc).unwrap_err();
        assert!(err.to_string().contains("verify"), "{err}");
    }

    #[test]
    fn rejects_truncated_and_empty_docs() {
        for doc in [&NITRO_DOC[..NITRO_DOC.len() / 2], &[][..]] {
            let err = verify_document(doc).unwrap_err();
            assert!(err.to_string().contains("decode"), "{err}");
        }
    }
}
