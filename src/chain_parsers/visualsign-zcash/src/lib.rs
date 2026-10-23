//! Zcash PCZT (partially created Zcash transaction) parser for VisualSign.
//!
//! The parser has no keys. It shows what a PCZT says and checks everything
//! that can be checked without a viewing key:
//!
//! - Every shielded output's recipient, value and `rseed` must reproduce the
//!   output's note commitment, which the ZIP-244 sighash commits to. Redacted
//!   outputs are refused.
//! - Memos are recovered from each output's own note fields (the sender-side
//!   ephemeral secret is derived from `rseed`), so no viewing key is needed.
//!   Nonzero outputs that do not recover are refused to avoid undetectable notes.
//! - Per pool, the value spent is the committed value balance plus the
//!   (commitment-checked) outputs. If the PCZT states spend values, they must
//!   add up to the same total.
//! - A `user_address` on an output must decode for the PCZT's network and
//!   contain the output's receiver.
//! - The fee is transparent in minus transparent out plus the shielded value
//!   balances, compared with the ZIP-317 conventional fee. Transparent input
//!   amounts are committed by the ZIP-244 sighashes. Transparent signers must
//!   use their per-input digest, not the shielded digest.
//! - Zero-value outputs whose memo is empty or does not decrypt are padding
//!   (ZIP 326 same-address outputs carry random ciphertexts) and are counted,
//!   not listed.
//!
//! Without a viewing key the parser cannot tell which spends belong to the
//! signer or which outputs are change. Spends are shown as pool totals and
//! change is marked "unverified (parser has no viewing key)"; every output,
//! including any change, is listed. Ironwood (NU6.3) bundles get the same
//! handling as Orchard.
//!
//! Shielded and per-transparent-input ZIP-244 sighashes are returned so a
//! wallet can bind its signatures to the rendered payload. Only SIGHASH_ALL
//! transparent inputs are accepted. The declared network is unsigned metadata.
//!
//! Resource policy: at most 4 MiB of encoded PCZT bytes and 64 total Sapling
//! outputs plus Orchard/Ironwood actions (including padding) are accepted.
//! The aggregate count is checked before sighashes, note recovery, and rendering.

#[cfg(feature = "cli-plugin")]
pub mod cli_plugin;

#[cfg(feature = "cli-plugin")]
pub use cli_plugin::{ZcashArgs, ZcashPlugin};

use base64::{Engine as _, engine::general_purpose::STANDARD as b64};
use orchard::note_encryption::{IronwoodDomain, OrchardDomain};
use pczt::Pczt;
use pczt::roles::verifier::{OrchardError, SaplingError, TransparentError, Verifier};
use visualsign::encodings::SupportedEncodings;
use visualsign::field_builders::{
    create_address_field, create_amount_field, create_number_field, create_text_field,
};
use visualsign::vsptrait::{
    ConversionResult, Transaction, TransactionParseError, VisualSignConverter,
    VisualSignConverterFromString, VisualSignError, VisualSignOptions,
};
use visualsign::{AnnotatedPayloadField, SignablePayload};
use zcash_keys::address::{Address, UnifiedAddress};
use zcash_keys::encoding::AddressCodec;
use zcash_note_encryption::{Domain, EphemeralKeyBytes, ShieldedOutput};
use zcash_primitives::transaction::fees::{FeeRule, transparent::InputSize, zip317};
use zcash_protocol::consensus::{BlockHeight, BranchId, Network};
use zcash_protocol::memo::{Memo, MemoBytes};
use zcash_script::script::Evaluable;
use zcash_transparent::address::TransparentAddress;

/// Largest PCZT accepted, in bytes.
pub const MAX_PCZT_BYTES: usize = 4 * 1024 * 1024;

/// Maximum total Sapling outputs plus Orchard and Ironwood actions, including
/// zero-value padding. This parser policy bounds per-output commitment checks,
/// note recovery, and rendering to 64 output records per request, while allowing
/// batch payments. It is independent of the byte cap and Zcash consensus rules.
pub const MAX_SHIELDED_OUTPUTS: usize = 64;

/// Text shown wherever the parser would need a viewing key to say more.
pub const UNVERIFIED: &str = "unverified (parser has no viewing key)";

const ZEC: &str = "ZEC";
const COIN_TYPE_MAIN: u32 = 133;
const COIN_TYPE_TEST: u32 = 1;

/// A parsed PCZT.
#[derive(Debug, Clone)]
pub struct ZcashTransactionWrapper {
    pczt: Pczt,
}

impl ZcashTransactionWrapper {
    /// Parse a PCZT from its encoded bytes. Trailing data and non-canonical
    /// encodings are refused: re-encoding under the input's own format version
    /// must reproduce the input exactly. Both v1 and v2 encodings are accepted.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, TransactionParseError> {
        if bytes.len() > MAX_PCZT_BYTES {
            return Err(TransactionParseError::InvalidFormat(format!(
                "PCZT is {} bytes; the limit is {MAX_PCZT_BYTES}",
                bytes.len()
            )));
        }
        let pczt = Pczt::parse(bytes)
            .map_err(|e| TransactionParseError::DecodeError(format!("PCZT: {e:?}")))?;
        // Check the shared work budget before cloning/re-encoding the PCZT,
        // computing sighashes, or doing any commitment checks/note recovery.
        // Count padding too: its note fields and ciphertext still need checking.
        let shielded_outputs = pczt
            .sapling()
            .outputs()
            .len()
            .saturating_add(pczt.orchard().actions().len())
            .saturating_add(pczt.ironwood().actions().len());
        if shielded_outputs > MAX_SHIELDED_OUTPUTS {
            return Err(TransactionParseError::InvalidFormat(format!(
                "PCZT has {shielded_outputs} shielded outputs/actions; the limit is {MAX_SHIELDED_OUTPUTS}"
            )));
        }
        let version = bytes
            .get(4..8)
            .and_then(|v| <[u8; 4]>::try_from(v).ok())
            .map(u32::from_le_bytes);
        let reencoded = match version {
            Some(1) => pczt::v1::Pczt::try_from(pczt.clone())
                .map(|p| p.serialize())
                .ok(),
            Some(2) => pczt::v2::Pczt::try_from(pczt.clone())
                .map(|p| p.serialize())
                .ok(),
            _ => None,
        };
        if reencoded.as_deref() != Some(bytes) {
            return Err(TransactionParseError::InvalidFormat(
                "non-canonical PCZT encoding".into(),
            ));
        }
        Ok(Self { pczt })
    }

    /// The parsed PCZT.
    pub fn pczt(&self) -> &Pczt {
        &self.pczt
    }
}

impl Transaction for ZcashTransactionWrapper {
    fn from_string(data: &str) -> Result<Self, TransactionParseError> {
        let data = data.trim();
        let bytes = match SupportedEncodings::detect(data) {
            SupportedEncodings::Hex => visualsign::encodings::decode_hex(data)
                .map_err(|e| TransactionParseError::DecodeError(format!("hex: {e}")))?,
            SupportedEncodings::Base64 => b64
                .decode(data)
                .map_err(|e| TransactionParseError::DecodeError(format!("base64: {e}")))?,
        };
        Self::from_bytes(&bytes)
    }

    fn transaction_type(&self) -> String {
        "Zcash".to_string()
    }
}

/// Converter for Zcash PCZTs.
pub struct ZcashVisualSignConverter;

impl VisualSignConverter<ZcashTransactionWrapper> for ZcashVisualSignConverter {
    fn to_visual_sign_payload(
        &self,
        transaction: ZcashTransactionWrapper,
        options: VisualSignOptions,
    ) -> Result<ConversionResult, VisualSignError> {
        let fields = decode(transaction.pczt)?;
        let title = options
            .transaction_name
            .unwrap_or_else(|| "Zcash Transaction".to_string());
        Ok(ConversionResult::new(SignablePayload::new(
            0,
            title,
            None,
            fields
                .into_iter()
                .map(|f| f.signable_payload_field)
                .collect(),
            "ZcashTx".to_string(),
        )))
    }
}

impl VisualSignConverterFromString<ZcashTransactionWrapper> for ZcashVisualSignConverter {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pool {
    Orchard,
    Ironwood,
    Sapling,
}

impl Pool {
    fn name(self) -> &'static str {
        match self {
            Self::Orchard => "Orchard",
            Self::Ironwood => "Ironwood",
            Self::Sapling => "Sapling",
        }
    }
}

enum MemoView {
    Empty,
    Text(String),
    Bytes(String),
    Unreadable,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn memo_view(bytes: Option<&[u8; 512]>) -> MemoView {
    let Some(bytes) = bytes else {
        return MemoView::Unreadable;
    };
    let Ok(mb) = MemoBytes::from_bytes(bytes) else {
        return MemoView::Unreadable;
    };
    let raw = mb.as_slice();
    let trimmed = &raw[..raw.iter().rposition(|b| *b != 0).map_or(0, |i| i + 1)];
    match Memo::try_from(&mb) {
        Ok(Memo::Empty) => MemoView::Empty,
        Ok(Memo::Text(t)) => {
            let t = String::from(t);
            if t.is_empty() {
                MemoView::Empty
            } else if t.bytes().all(|b| b == b' ' || b.is_ascii_graphic()) {
                MemoView::Text(t)
            } else {
                // Wallet text fields are printable ASCII only; show the UTF-8
                // bytes instead of a lossy rendering.
                MemoView::Bytes(format!("text memo, UTF-8 0x{}", hex(trimmed)))
            }
        }
        _ => MemoView::Bytes(format!("binary memo 0x{}", hex(trimmed))),
    }
}

struct Send {
    to: CheckedAddress,
    kind: String,
    value: u64,
    memo: MemoView,
}

#[derive(Default)]
struct Analysis {
    sends: Vec<Send>,
    spent: Vec<(Pool, u64)>,
    t_inputs: Vec<(String, u64)>,
    t_in: u64,
    t_out: u64,
    /// `None` once any input's size is unknown (non-P2PKH).
    t_input_sizes: Option<Vec<InputSize>>,
    t_output_sizes: Vec<usize>,
    /// Per shielded pool: (pool, sum of output values, sum of stated spend
    /// values if every spend states one).
    pool_outputs: Vec<(Pool, i128, Option<i128>)>,
    dummies: usize,
    sapling_proof_key: bool,
}

fn malformed(s: impl Into<String>) -> VisualSignError {
    VisualSignError::DecodeError(s.into())
}

/// Zatoshis as a decimal ZEC string with 8 places.
fn zec(z: u64) -> String {
    format!("{}.{:08}", z / 100_000_000, z % 100_000_000)
}

fn add(total: &mut u64, v: u64) -> Result<(), VisualSignError> {
    *total = total
        .checked_add(v)
        .ok_or_else(|| malformed("value overflow"))?;
    Ok(())
}

struct CheckedAddress {
    text: String,
    expiry_time: Option<u64>,
    tex: bool,
}

/// Check a `user_address` against the output's receiver and return the text
/// to display.
fn check_user_address(
    net: &Network,
    tx_expiry: u32,
    user: &Option<String>,
    matches: impl Fn(&Address) -> bool,
    fallback: impl FnOnce() -> Option<String>,
) -> Result<CheckedAddress, VisualSignError> {
    match user {
        Some(s) => {
            let addr = Address::decode(net, s).ok_or_else(|| {
                VisualSignError::ValidationError(format!(
                    "user address {s} is not a valid address on this network"
                ))
            })?;
            if !matches(&addr) {
                return Err(VisualSignError::ValidationError(format!(
                    "user address {s} does not contain the output's receiver"
                )));
            }
            let expiry_time = if let Address::Unified(ua) = &addr {
                if ua
                    .expiry_height()
                    .is_some_and(|h| tx_expiry == 0 || tx_expiry > u32::from(h))
                {
                    return Err(VisualSignError::ValidationError(
                        "transaction expiry exceeds user address expiry height (or is unlimited)"
                            .into(),
                    ));
                }
                ua.expiry_time()
            } else {
                None
            };
            Ok(CheckedAddress {
                text: s.clone(),
                expiry_time,
                tex: matches!(addr, Address::Tex(_)),
            })
        }
        None => Ok(CheckedAddress {
            text: fallback().ok_or_else(|| malformed("cannot encode receiver"))?,
            expiry_time: None,
            tex: false,
        }),
    }
}

fn orchard_ua(net: &Network, addr: orchard::Address) -> Option<String> {
    UnifiedAddress::from_receivers(Some(addr), None, None, None, None).map(|ua| ua.encode(net))
}

/// Recover an Orchard-protocol output's memo from its own note fields: the
/// ephemeral secret derives from `rseed`, so the sender's side of the key
/// agreement is available without any viewing key.
fn orchard_memo(pool: Pool, action: &orchard::pczt::Action) -> Option<[u8; 512]> {
    let out = action.output();
    let rho =
        orchard::note::Rho::from_bytes(&action.spend().nullifier().to_bytes()).into_option()?;
    let note = orchard::Note::from_parts(
        (*out.recipient())?,
        (*out.value())?,
        rho,
        (*out.rseed())?,
        *out.note_version(),
    )
    .into_option()?;
    macro_rules! recover {
        ($d:ty, $domain:expr) => {{
            let esk = <$d as Domain>::derive_esk(&note)?;
            let pk_d = <$d as Domain>::get_pk_d(&note);
            zcash_note_encryption::try_output_recovery_with_pkd_esk(&$domain, pk_d, esk, action)
                .map(|(_, _, m)| m)
        }};
    }
    match pool {
        Pool::Ironwood => recover!(IronwoodDomain, IronwoodDomain::for_pczt_action(action)),
        _ => recover!(OrchardDomain, OrchardDomain::for_pczt_action(action)),
    }
}

fn analyze_orchard(
    pool: Pool,
    net: &Network,
    expiry: u32,
    bundle: &orchard::pczt::Bundle,
    a: &mut Analysis,
) -> Result<(), VisualSignError> {
    let p = pool.name();
    bundle
        .verify_cross_address_restriction()
        .map_err(|e| malformed(format!("{p} cross-address restriction: {e:?}")))?;
    let mut outputs: i128 = 0;
    let mut stated_spends: Option<i128> = Some(0);
    for (i, action) in bundle.actions().iter().enumerate() {
        let spend = action.spend();
        if spend.dummy_sk().is_some() {
            return Err(malformed(format!(
                "{p} action {i}: unfinalized dummy spend"
            )));
        }
        stated_spends = match (stated_spends, spend.value()) {
            (Some(s), Some(v)) => Some(s + i128::from(v.inner())),
            _ => None,
        };

        let out = action.output();
        let redacted = || VisualSignError::MissingData(format!("{p} action {i}: output redacted"));
        let recipient = out.recipient().ok_or_else(redacted)?;
        let value = out.value().ok_or_else(redacted)?.inner();
        out.verify_note_commitment(spend).map_err(|e| {
            malformed(format!(
                "{p} action {i}: output does not match its note commitment ({e:?})"
            ))
        })?;
        outputs += i128::from(value);

        let shown = check_user_address(
            net,
            expiry,
            out.user_address(),
            |addr| matches!(addr, Address::Unified(ua) if ua.orchard() == Some(&recipient)),
            || orchard_ua(net, recipient),
        )?;
        let recovered = orchard_memo(pool, action);
        if value > 0 && recovered.is_none() {
            return Err(malformed(format!(
                "{p} action {i}: nonzero output does not recover from its note fields"
            )));
        }
        let memo = memo_view(recovered.as_ref());
        if value == 0 && matches!(memo, MemoView::Empty | MemoView::Unreadable) {
            a.dummies += 1;
            continue;
        }
        a.sends.push(Send {
            to: shown,
            kind: p.to_string(),
            value,
            memo,
        });
    }
    a.pool_outputs.push((pool, outputs, stated_spends));
    Ok(())
}

/// Sapling PCZT output viewed as a `ShieldedOutput` for note decryption.
struct SaplingOut {
    epk: EphemeralKeyBytes,
    cmu: sapling_crypto::note::ExtractedNoteCommitment,
    enc: sapling_crypto::note_encryption::NoteCiphertextBytes,
}

impl ShieldedOutput<sapling_crypto::note_encryption::SaplingDomain> for SaplingOut {
    fn ephemeral_key(&self) -> EphemeralKeyBytes {
        self.epk.clone()
    }
    fn cmstar(&self) -> &sapling_crypto::note::ExtractedNoteCommitment {
        &self.cmu
    }
    fn enc_ciphertext(&self) -> Option<&sapling_crypto::note_encryption::NoteCiphertextBytes> {
        Some(&self.enc)
    }
    fn enc_ciphertext_compact(
        &self,
    ) -> sapling_crypto::note_encryption::CompactNoteCiphertextBytes {
        let mut c = [0u8; sapling_crypto::note_encryption::COMPACT_NOTE_SIZE];
        c.copy_from_slice(&self.enc.as_ref()[..sapling_crypto::note_encryption::COMPACT_NOTE_SIZE]);
        zcash_note_encryption::note_bytes::NoteBytesData(c)
    }
}

fn analyze_sapling(
    net: &Network,
    expiry: u32,
    bundle: &sapling_crypto::pczt::Bundle,
    a: &mut Analysis,
) -> Result<(), VisualSignError> {
    use sapling_crypto::note_encryption::{SaplingDomain, Zip212Enforcement};
    let mut stated_spends: Option<i128> = Some(0);
    for (i, spend) in bundle.spends().iter().enumerate() {
        if spend.dummy_ask().is_some() {
            return Err(malformed(format!(
                "Sapling spend {i}: unfinalized dummy spend"
            )));
        }
        stated_spends = match (stated_spends, spend.value()) {
            (Some(s), Some(v)) => Some(s + i128::from(v.inner())),
            _ => None,
        };
        a.sapling_proof_key |= spend.proof_generation_key().is_some();
    }
    let domain = SaplingDomain::new(Zip212Enforcement::On);
    let mut outputs: i128 = 0;
    for (i, out) in bundle.outputs().iter().enumerate() {
        let redacted = || VisualSignError::MissingData(format!("Sapling output {i}: redacted"));
        let recipient = out.recipient().ok_or_else(redacted)?;
        let value = out.value().ok_or_else(redacted)?.inner();
        let rseed = out.rseed().ok_or_else(redacted)?;
        out.verify_note_commitment().map_err(|e| {
            malformed(format!(
                "Sapling output {i}: does not match its note commitment ({e:?})"
            ))
        })?;
        outputs += i128::from(value);
        let shown = check_user_address(
            net,
            expiry,
            out.user_address(),
            |addr| match addr {
                Address::Sapling(pa) => **pa == recipient,
                Address::Unified(ua) => ua.sapling() == Some(&recipient),
                _ => false,
            },
            || Some(recipient.encode(net)),
        )?;
        let so = SaplingOut {
            epk: out.ephemeral_key().clone(),
            cmu: *out.cmu(),
            enc: zcash_note_encryption::note_bytes::NoteBytesData(*out.enc_ciphertext()),
        };
        let note = sapling_crypto::Note::from_parts(
            recipient,
            sapling_crypto::value::NoteValue::from_raw(value),
            sapling_crypto::Rseed::AfterZip212(rseed),
        );
        let memo = SaplingDomain::derive_esk(&note).and_then(|esk| {
            zcash_note_encryption::try_output_recovery_with_pkd_esk(
                &domain,
                SaplingDomain::get_pk_d(&note),
                esk,
                &so,
            )
            .map(|(_, _, m)| m)
        });
        if value > 0 && memo.is_none() {
            return Err(malformed(format!(
                "Sapling output {i}: nonzero output does not recover from its note fields"
            )));
        }
        let memo = memo_view(memo.as_ref());
        if value == 0 && matches!(memo, MemoView::Empty | MemoView::Unreadable) {
            a.dummies += 1;
            continue;
        }
        a.sends.push(Send {
            to: shown,
            kind: "Sapling".into(),
            value,
            memo,
        });
    }
    a.pool_outputs.push((Pool::Sapling, outputs, stated_spends));
    Ok(())
}

fn compact_size_len(n: usize) -> usize {
    match n {
        0..=0xfc => 1,
        0xfd..=0xffff => 3,
        _ => 5,
    }
}

fn analyze_transparent(
    net: &Network,
    expiry: u32,
    bundle: &zcash_transparent::pczt::Bundle,
    a: &mut Analysis,
) -> Result<(), VisualSignError> {
    a.t_input_sizes = Some(vec![]);
    for input in bundle.inputs() {
        let value = u64::from(*input.value());
        add(&mut a.t_in, value)?;
        let addr = TransparentAddress::from_script_from_chain(input.script_pubkey());
        let (shown, p2pkh) = match addr {
            Some(t @ TransparentAddress::PublicKeyHash(_)) => {
                (format!("{} (P2PKH)", t.encode(net)), true)
            }
            Some(t @ TransparentAddress::ScriptHash(_)) => {
                (format!("{} (P2SH)", t.encode(net)), false)
            }
            None => (
                format!(
                    "script 0x{} (non-standard)",
                    hex(&input.script_pubkey().to_bytes())
                ),
                false,
            ),
        };
        match (&mut a.t_input_sizes, p2pkh) {
            (Some(sizes), true) => sizes.push(InputSize::STANDARD_P2PKH),
            // The spending script size of a P2SH or non-standard input is not
            // known, so the ZIP-317 conventional fee cannot be computed.
            _ => a.t_input_sizes = None,
        }
        a.t_inputs.push((shown, value));
    }
    for out in bundle.outputs() {
        let value = u64::from(*out.value());
        add(&mut a.t_out, value)?;
        let script = out.script_pubkey();
        let len = script.byte_len();
        a.t_output_sizes.push(8 + compact_size_len(len) + len);
        let addr = TransparentAddress::from_script_pubkey(script);
        let claims_change = !out.bip32_derivation().is_empty();
        let (to, kind) = match addr {
            Some(t) => {
                let shown = check_user_address(
                    net,
                    expiry,
                    out.user_address(),
                    |u| match u {
                        Address::Transparent(x) => *x == t,
                        Address::Unified(ua) => ua.transparent() == Some(&t),
                        Address::Tex(h) => t == TransparentAddress::PublicKeyHash(*h),
                        _ => false,
                    },
                    || Some(t.encode(net)),
                )?;
                let kind = match t {
                    TransparentAddress::PublicKeyHash(_) => "transparent P2PKH",
                    TransparentAddress::ScriptHash(_) => "transparent P2SH",
                };
                (shown, kind.to_string())
            }
            None => (
                CheckedAddress {
                    text: format!("script 0x{}", hex(&script.to_bytes())),
                    expiry_time: None,
                    tex: false,
                },
                "non-standard transparent script".to_string(),
            ),
        };
        let kind = if claims_change {
            format!("{kind}, claimed change {UNVERIFIED}")
        } else {
            kind
        };
        a.sends.push(Send {
            to,
            kind,
            value,
            memo: MemoView::Empty,
        });
    }
    Ok(())
}

fn orchard_err(e: OrchardError<VisualSignError>) -> VisualSignError {
    match e {
        OrchardError::Custom(d) => d,
        other => malformed(format!("Orchard bundle: {other:?}")),
    }
}

fn sapling_err(e: SaplingError<VisualSignError>) -> VisualSignError {
    match e {
        SaplingError::Custom(d) => d,
        other => malformed(format!("Sapling bundle: {other:?}")),
    }
}

fn transparent_err(e: TransparentError<VisualSignError>) -> VisualSignError {
    match e {
        TransparentError::Custom(d) => d,
        other => malformed(format!("transparent bundle: {other:?}")),
    }
}

fn network(coin_type: u32) -> Result<Network, VisualSignError> {
    match coin_type {
        COIN_TYPE_MAIN => Ok(Network::MainNetwork),
        COIN_TYPE_TEST => Ok(Network::TestNetwork),
        other => Err(malformed(format!("unknown coin type {other}"))),
    }
}

fn decode(pczt: Pczt) -> Result<Vec<AnnotatedPayloadField>, VisualSignError> {
    let g = pczt.global();
    let net = network(*g.coin_type())?;
    let branch = BranchId::try_from(*g.consensus_branch_id())
        .map_err(|_| malformed("unknown consensus branch"))?;
    let expiry = *g.expiry_height();

    let signer = pczt::roles::signer::Signer::new(pczt.clone())
        .map_err(|e| malformed(format!("cannot compute sighash: {e:?}")))?;
    // The default ALL_ONLY policy refuses NONE, SINGLE and ANYONECANPAY.
    // Compute every transparent digest before rendering any destinations.
    let transparent_sighashes = (0..pczt.transparent().inputs().len())
        .map(|i| {
            signer
                .transparent_sighash(i)
                .map_err(|e| malformed(format!("transparent input {} sighash: {e:?}", i + 1)))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let sighash = signer.shielded_sighash();
    let effects = pczt
        .clone()
        .into_effects()
        .map_err(|e| malformed(format!("cannot extract effects: {e:?}")))?;

    let mut a = Analysis::default();
    let v = Verifier::new(pczt)
        .with_orchard(|b| {
            analyze_orchard(Pool::Orchard, &net, expiry, b, &mut a).map_err(OrchardError::Custom)
        })
        .map_err(orchard_err)?
        .with_ironwood(|b| {
            analyze_orchard(Pool::Ironwood, &net, expiry, b, &mut a).map_err(OrchardError::Custom)
        })
        .map_err(orchard_err)?
        .with_sapling(|b| analyze_sapling(&net, expiry, b, &mut a).map_err(SaplingError::Custom))
        .map_err(sapling_err)?;
    let _ = v
        .with_transparent(|b| {
            analyze_transparent(&net, expiry, b, &mut a).map_err(TransparentError::Custom)
        })
        .map_err(transparent_err)?;

    // Per pool, the committed value balance plus the commitment-checked
    // outputs is what the spends add up to. Spend values stated in the PCZT
    // must agree.
    let balance = |pool: Pool| -> i128 {
        i128::from(match pool {
            Pool::Orchard => effects
                .orchard_bundle()
                .map_or(0, |b| i64::from(*b.value_balance())),
            Pool::Ironwood => effects
                .ironwood_bundle()
                .map_or(0, |b| i64::from(*b.value_balance())),
            Pool::Sapling => effects
                .sapling_bundle()
                .map_or(0, |b| i64::from(*b.value_balance())),
        })
    };
    let mut shielded_balance: i128 = 0;
    for (pool, outputs, stated) in std::mem::take(&mut a.pool_outputs) {
        let vb = balance(pool);
        shielded_balance += vb;
        let spent = vb + outputs;
        if spent < 0 || stated.is_some_and(|s| s != spent) {
            return Err(malformed(format!(
                "{} value balance does not match the notes",
                pool.name()
            )));
        }
        let spent = u64::try_from(spent).map_err(|_| malformed("value overflow"))?;
        if spent > 0 {
            a.spent.push((pool, spent));
        }
    }
    if !a.spent.is_empty() && a.sends.iter().any(|s| s.to.tex) {
        return Err(VisualSignError::ValidationError(
            "TEX recipients require transparent-only sources; transaction spends shielded funds"
                .into(),
        ));
    }
    let fee = i128::from(a.t_in) - i128::from(a.t_out) + shielded_balance;
    let fee = u64::try_from(fee).map_err(|_| malformed("outputs exceed inputs"))?;

    let count = |n: Option<usize>| n.unwrap_or(0);
    let conventional = match (&a.t_input_sizes, expiry) {
        (_, 0) => None,
        (sizes, _) => zip317::FeeRule::standard()
            .fee_required(
                &net,
                BlockHeight::from_u32(expiry),
                sizes.clone().unwrap_or_default(),
                a.t_output_sizes.iter().copied(),
                count(effects.sapling_bundle().map(|b| b.shielded_spends().len())),
                count(effects.sapling_bundle().map(|b| b.shielded_outputs().len())),
                count(effects.orchard_bundle().map(|b| b.actions().len())),
                count(effects.ironwood_bundle().map(|b| b.actions().len())),
            )
            .map(u64::from)
            .ok()
            .filter(|_| sizes.is_some() || a.t_inputs.is_empty()),
    };

    let mut fields = vec![create_text_field(
        "Declared network (unsigned)",
        match net {
            Network::MainNetwork => "Zcash mainnet",
            Network::TestNetwork => "Zcash testnet",
        },
    )?];
    fields.push(create_text_field(
        "Network trust",
        "PCZT coin type and address prefixes are unsigned; confirm the network independently",
    )?);
    for (n, s) in a.sends.iter().enumerate() {
        let n = n + 1;
        fields.push(create_address_field(
            &format!("Output {n}"),
            &s.to.text,
            None,
            None,
            None,
            Some(&s.kind),
        )?);
        fields.push(create_amount_field(
            &format!("Output {n} amount"),
            &zec(s.value),
            ZEC,
        )?);
        if let Some(time) = s.to.expiry_time {
            fields.push(create_text_field(
                &format!("Output {n} address expiry time"),
                &format!("{time} Unix seconds; not checked (parser has no clock)"),
            )?);
        }
        let memo = match &s.memo {
            MemoView::Empty => None,
            MemoView::Text(t) => Some(t.clone()),
            MemoView::Bytes(b) => Some(b.clone()),
            MemoView::Unreadable => Some("does not decrypt from the PCZT's note fields".into()),
        };
        if let Some(text) = memo {
            fields.push(create_text_field(&format!("Output {n} memo"), &text)?);
        }
    }
    fields.push(create_text_field(
        "Change",
        &format!("{UNVERIFIED}; outputs back to the sender are listed as outputs"),
    )?);
    for (pool, v) in &a.spent {
        fields.push(create_text_field(
            &format!("{} spent", pool.name()),
            &format!("{} ZEC; spend ownership {UNVERIFIED}", zec(*v)),
        )?);
    }
    for (i, (addr, value)) in a.t_inputs.iter().enumerate() {
        let text = format!("{addr} {} ZEC", zec(*value));
        fields.push(create_text_field(
            &format!("Transparent input {}", i + 1),
            &text,
        )?);
    }
    for (i, digest) in transparent_sighashes.iter().enumerate() {
        fields.push(create_text_field(
            &format!("Transparent input {} sighash (ZIP-244)", i + 1),
            &hex(digest),
        )?);
    }
    fields.push(create_amount_field("Fee", &zec(fee), ZEC)?);
    let fee_check = match conventional {
        Some(c) if c == fee => "equals the ZIP-317 conventional fee".to_string(),
        Some(c) if fee > c => format!("above the ZIP-317 conventional fee of {} ZEC", zec(c)),
        Some(c) => format!(
            "below the ZIP-317 conventional fee of {} ZEC; may not be relayed",
            zec(c)
        ),
        None => {
            "ZIP-317 conventional fee not computed (unknown input sizes or no expiry)".to_string()
        }
    };
    fields.push(create_text_field("Fee check", &fee_check)?);
    if a.dummies > 0 {
        fields.push(create_number_field(
            "Padding outputs",
            &a.dummies.to_string(),
            "",
        )?);
    }
    if a.sapling_proof_key {
        fields.push(create_text_field(
            "Sapling proof key",
            "the PCZT carries a Sapling proof generation key (lets its holder prove and link the spender's Sapling spends, not spend them)",
        )?);
    }
    fields.push(create_number_field(
        "Expiry height",
        &expiry.to_string(),
        "",
    )?);
    fields.push(create_text_field(
        "Consensus branch",
        &format!("{branch:?} (0x{:08x})", u32::from(branch)),
    )?);
    fields.push(create_text_field(
        "Shielded sighash (ZIP-244)",
        &hex(sighash.as_ref()),
    )?);
    Ok(fields)
}
