//! Borsh wire types for the NEAR transactions this crate decodes.
//!
//! These mirror `near-primitives` 0.35.1 and `near-crypto` 0.35.1 byte for
//! byte -- field order, enum discriminants, and every hand-written
//! `BorshDeserialize` impl -- so that an input decodes here exactly when it
//! decodes upstream, with the same error message when it does not.
//! `near-primitives` 0.35.1 makes `zstd` mandatory, so depending on it linked
//! `zstd-sys` (native C) into every embedder for what is only a decoder.
//! `tests/primitives_differential.rs` and `tests/primitives_semantic.rs` check
//! the equivalence against the upstream crates, which are dev-dependencies
//! only.
//!
//! This does not remove `near-crypto` or `secp256k1-sys` 0.8 from the build:
//! both still arrive through `near-sdk`'s `non-contract-usage` feature, which
//! near-sdk 5.27 requires to compile on a non-wasm target and which the
//! `defuse-*` intents crates use.
//!
//! Account ids, token amounts and gas reuse `near-sdk`'s re-exports of
//! `near-account-id`, `near-token` and `near-gas` -- the same crates
//! `near-primitives` builds on -- so account-id validation (borsh and serde) is
//! upstream's own code rather than a copy.
//!
//! Only decoding and re-encoding are supported: nothing here verifies a
//! signature or computes a transaction hash.

use std::collections::BTreeMap;
use std::fmt;
use std::io::{Error, ErrorKind, Read, Write};

use borsh::{BorshDeserialize, BorshSerialize};

pub use near_sdk::{AccountId, Gas, NearToken as Balance};

/// `near_primitives::types::Nonce`.
pub type Nonce = u64;
/// `near_primitives::types::NonceIndex`.
pub type NonceIndex = u16;
/// `near_primitives::types::BlockHeight`.
pub type BlockHeight = u64;

/// `near_primitives::hash::CryptoHash`: 32 raw bytes on the wire.
#[derive(
    BorshSerialize, BorshDeserialize, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
pub struct CryptoHash(pub [u8; 32]);

impl fmt::Debug for CryptoHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&bs58::encode(self.0).into_string())
    }
}

// ---------------------------------------------------------------------------
// Keys and signatures (near-crypto 0.35.1, signature.rs)
// ---------------------------------------------------------------------------

/// The curve tag that prefixes a key or signature on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyType {
    ED25519 = 0,
    SECP256K1 = 1,
}

impl KeyType {
    /// Upstream's `KeyType::try_from(u8)`, with its error text.
    fn from_tag(tag: u8) -> std::io::Result<Self> {
        match tag {
            0 => Ok(Self::ED25519),
            1 => Ok(Self::SECP256K1),
            unknown => Err(Error::new(
                ErrorKind::InvalidData,
                format!("unknown key type '{unknown}'"),
            )),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::ED25519 => "ed25519",
            Self::SECP256K1 => "secp256k1",
        }
    }
}

/// `near_crypto::PublicKey`: a curve tag byte, then 32 (ed25519) or 64
/// (secp256k1, uncompressed without the `0x04` prefix) raw bytes. No length
/// prefix, and no on-curve check at decode time.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PublicKey {
    ED25519([u8; 32]),
    SECP256K1([u8; 64]),
}

impl PublicKey {
    /// An all-zero key of the given type, as `near_crypto::PublicKey::empty`.
    #[must_use]
    pub fn empty(key_type: KeyType) -> Self {
        match key_type {
            KeyType::ED25519 => Self::ED25519([0; 32]),
            KeyType::SECP256K1 => Self::SECP256K1([0; 64]),
        }
    }

    #[must_use]
    pub fn key_type(&self) -> KeyType {
        match self {
            Self::ED25519(_) => KeyType::ED25519,
            Self::SECP256K1(_) => KeyType::SECP256K1,
        }
    }

    #[must_use]
    pub fn key_data(&self) -> &[u8] {
        match self {
            Self::ED25519(key) => key,
            Self::SECP256K1(key) => key,
        }
    }
}

/// `"{ed25519|secp256k1}:{base58}"`, as near-crypto renders it.
impl fmt::Display for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}",
            self.key_type().name(),
            bs58::encode(self.key_data()).into_string()
        )
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl BorshSerialize for PublicKey {
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        BorshSerialize::serialize(&(self.key_type() as u8), writer)?;
        writer.write_all(self.key_data())
    }
}

impl BorshDeserialize for PublicKey {
    fn deserialize_reader<R: Read>(rd: &mut R) -> std::io::Result<Self> {
        match KeyType::from_tag(u8::deserialize_reader(rd)?)? {
            KeyType::ED25519 => Ok(Self::ED25519(BorshDeserialize::deserialize_reader(rd)?)),
            KeyType::SECP256K1 => Ok(Self::SECP256K1(BorshDeserialize::deserialize_reader(rd)?)),
        }
    }
}

/// `near_crypto::Signature`: a curve tag byte, then 64 (ed25519) or 65
/// (secp256k1, `r || s || v`) raw bytes.
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum Signature {
    ED25519([u8; 64]),
    SECP256K1([u8; 65]),
}

impl Signature {
    /// An all-zero signature of the given type, as
    /// `near_crypto::Signature::empty`.
    #[must_use]
    pub fn empty(key_type: KeyType) -> Self {
        match key_type {
            KeyType::ED25519 => Self::ED25519([0; 64]),
            KeyType::SECP256K1 => Self::SECP256K1([0; 65]),
        }
    }

    #[must_use]
    pub fn key_type(&self) -> KeyType {
        match self {
            Self::ED25519(_) => KeyType::ED25519,
            Self::SECP256K1(_) => KeyType::SECP256K1,
        }
    }

    fn data(&self) -> &[u8] {
        match self {
            Self::ED25519(sig) => sig,
            Self::SECP256K1(sig) => sig,
        }
    }
}

impl fmt::Display for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}",
            self.key_type().name(),
            bs58::encode(self.data()).into_string()
        )
    }
}

impl fmt::Debug for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl BorshSerialize for Signature {
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        BorshSerialize::serialize(&(self.key_type() as u8), writer)?;
        writer.write_all(self.data())
    }
}

impl BorshDeserialize for Signature {
    fn deserialize_reader<R: Read>(rd: &mut R) -> std::io::Result<Self> {
        match KeyType::from_tag(u8::deserialize_reader(rd)?)? {
            KeyType::ED25519 => {
                let array: [u8; 64] = BorshDeserialize::deserialize_reader(rd)?;
                // near-crypto keeps the pre-dalek-2 sanity check: the top three
                // bits of an ed25519 signature's last byte must be clear.
                if array[63] & 0b1110_0000 != 0 {
                    return Err(Error::new(ErrorKind::InvalidData, "signature error"));
                }
                Ok(Self::ED25519(array))
            }
            KeyType::SECP256K1 => Ok(Self::SECP256K1(BorshDeserialize::deserialize_reader(rd)?)),
        }
    }
}

// ---------------------------------------------------------------------------
// Access keys (near-primitives-core 0.35.1, account.rs)
// ---------------------------------------------------------------------------

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct AccessKey {
    pub nonce: Nonce,
    pub permission: AccessKeyPermission,
}

/// Declaration order is the wire tag: FunctionCall=0, FullAccess=1,
/// GasKeyFunctionCall=2, GasKeyFullAccess=3.
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub enum AccessKeyPermission {
    FunctionCall(FunctionCallPermission),
    FullAccess,
    GasKeyFunctionCall(GasKeyInfo, FunctionCallPermission),
    GasKeyFullAccess(GasKeyInfo),
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct GasKeyInfo {
    pub balance: Balance,
    pub num_nonces: NonceIndex,
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct FunctionCallPermission {
    pub allowance: Option<Balance>,
    /// A plain string upstream, not an `AccountId`: testnet genesis holds
    /// records that fail account-id validation, so it is not validated.
    pub receiver_id: String,
    pub method_names: Vec<String>,
}

// ---------------------------------------------------------------------------
// Global contracts and deterministic accounts
// ---------------------------------------------------------------------------

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
#[borsh(use_discriminant = true)]
#[repr(u8)]
pub enum GlobalContractIdentifier {
    CodeHash(CryptoHash) = 0,
    AccountId(AccountId) = 1,
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum GlobalContractDeployMode {
    CodeHash,
    AccountId,
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
#[borsh(use_discriminant = true)]
#[repr(u8)]
pub enum DeterministicAccountStateInit {
    V1(DeterministicAccountStateInitV1),
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct DeterministicAccountStateInitV1 {
    pub code: GlobalContractIdentifier,
    /// borsh's own `BTreeMap` decoding, as upstream (no strict key-order
    /// check: `de_strict_order` is not enabled anywhere in the build).
    pub data: BTreeMap<Vec<u8>, Vec<u8>>,
}

// ---------------------------------------------------------------------------
// Actions (near-primitives 0.35.1, action/mod.rs and action/delegate.rs)
// ---------------------------------------------------------------------------

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct CreateAccountAction {}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct DeployContractAction {
    pub code: Vec<u8>,
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct FunctionCallAction {
    pub method_name: String,
    pub args: Vec<u8>,
    pub gas: Gas,
    pub deposit: Balance,
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct TransferAction {
    pub deposit: Balance,
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct StakeAction {
    pub stake: Balance,
    pub public_key: PublicKey,
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct AddKeyAction {
    pub public_key: PublicKey,
    pub access_key: AccessKey,
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct DeleteKeyAction {
    pub public_key: PublicKey,
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct DeleteAccountAction {
    pub beneficiary_id: AccountId,
}

/// Upstream holds `code` as `Arc<[u8]>`; the encoding (u32 length + bytes) is
/// the same as `Vec<u8>`.
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct DeployGlobalContractAction {
    pub code: Vec<u8>,
    pub deploy_mode: GlobalContractDeployMode,
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct UseGlobalContractAction {
    pub contract_identifier: GlobalContractIdentifier,
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct DeterministicStateInitAction {
    pub state_init: DeterministicAccountStateInit,
    pub deposit: Balance,
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct TransferToGasKeyAction {
    pub public_key: PublicKey,
    pub deposit: Balance,
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct WithdrawFromGasKeyAction {
    pub public_key: PublicKey,
    pub amount: Balance,
}

/// `near_primitives::action::Action`, with its explicit wire discriminants.
/// An unknown tag fails with borsh's `Unexpected variant tag`.
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
#[borsh(use_discriminant = true)]
#[repr(u8)]
pub enum Action {
    CreateAccount(CreateAccountAction) = 0,
    DeployContract(DeployContractAction) = 1,
    FunctionCall(Box<FunctionCallAction>) = 2,
    Transfer(TransferAction) = 3,
    Stake(Box<StakeAction>) = 4,
    AddKey(Box<AddKeyAction>) = 5,
    DeleteKey(Box<DeleteKeyAction>) = 6,
    DeleteAccount(DeleteAccountAction) = 7,
    Delegate(Box<SignedDelegateAction>) = 8,
    DeployGlobalContract(DeployGlobalContractAction) = 9,
    UseGlobalContract(Box<UseGlobalContractAction>) = 10,
    DeterministicStateInit(Box<DeterministicStateInitAction>) = 11,
    TransferToGasKey(Box<TransferToGasKeyAction>) = 12,
    WithdrawFromGasKey(Box<WithdrawFromGasKeyAction>) = 13,
}

/// Wire tag of [`Action::Delegate`].
const ACTION_DELEGATE_NUMBER: u8 = 8;

/// NEP-366 meta-transaction body.
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct DelegateAction {
    pub sender_id: AccountId,
    pub receiver_id: AccountId,
    pub actions: Vec<NonDelegateAction>,
    pub nonce: Nonce,
    pub max_block_height: BlockHeight,
    pub public_key: PublicKey,
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct SignedDelegateAction {
    pub delegate_action: DelegateAction,
    pub signature: Signature,
}

/// An [`Action`] that is not a delegate: a delegate action's inner list may
/// not nest another delegate, and upstream enforces that while decoding.
#[derive(BorshSerialize, Clone, Debug, PartialEq, Eq)]
pub struct NonDelegateAction(Action);

impl NonDelegateAction {
    #[must_use]
    pub fn action(&self) -> &Action {
        &self.0
    }
}

impl From<NonDelegateAction> for Action {
    fn from(action: NonDelegateAction) -> Self {
        action.0
    }
}

/// Returned when constructing a [`NonDelegateAction`] from a delegate.
#[derive(Debug, thiserror::Error)]
#[error("attempted to construct NonDelegateAction from Action::Delegate")]
pub struct IsDelegateAction;

impl TryFrom<Action> for NonDelegateAction {
    type Error = IsDelegateAction;

    fn try_from(action: Action) -> Result<Self, IsDelegateAction> {
        if matches!(action, Action::Delegate(_)) {
            Err(IsDelegateAction)
        } else {
            Ok(Self(action))
        }
    }
}

impl BorshDeserialize for NonDelegateAction {
    fn deserialize_reader<R: Read>(rd: &mut R) -> std::io::Result<Self> {
        match u8::deserialize_reader(rd)? {
            ACTION_DELEGATE_NUMBER => Err(Error::new(
                ErrorKind::InvalidInput,
                "DelegateAction mustn't contain a nested one",
            )),
            n => borsh::de::EnumExt::deserialize_variant(rd, n).map(Self),
        }
    }
}

// ---------------------------------------------------------------------------
// Transactions (near-primitives 0.35.1, transaction.rs)
// ---------------------------------------------------------------------------

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct TransactionV0 {
    pub signer_id: AccountId,
    pub public_key: PublicKey,
    pub nonce: Nonce,
    pub receiver_id: AccountId,
    pub block_hash: CryptoHash,
    pub actions: Vec<Action>,
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransactionNonce {
    Nonce {
        nonce: Nonce,
    },
    GasKeyNonce {
        nonce: Nonce,
        nonce_index: NonceIndex,
    },
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct TransactionV1 {
    pub signer_id: AccountId,
    pub public_key: PublicKey,
    pub nonce: TransactionNonce,
    pub receiver_id: AccountId,
    pub block_hash: CryptoHash,
    pub actions: Vec<Action>,
}

/// A NEAR transaction. V0 is encoded bare; V1 is prefixed with a `1u8` tag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Transaction {
    V0(TransactionV0),
    V1(TransactionV1),
}

impl Transaction {
    #[must_use]
    pub fn signer_id(&self) -> &AccountId {
        match self {
            Self::V0(tx) => &tx.signer_id,
            Self::V1(tx) => &tx.signer_id,
        }
    }

    #[must_use]
    pub fn receiver_id(&self) -> &AccountId {
        match self {
            Self::V0(tx) => &tx.receiver_id,
            Self::V1(tx) => &tx.receiver_id,
        }
    }

    #[must_use]
    pub fn public_key(&self) -> &PublicKey {
        match self {
            Self::V0(tx) => &tx.public_key,
            Self::V1(tx) => &tx.public_key,
        }
    }

    #[must_use]
    pub fn actions(&self) -> &[Action] {
        match self {
            Self::V0(tx) => &tx.actions,
            Self::V1(tx) => &tx.actions,
        }
    }

    #[must_use]
    pub fn block_hash(&self) -> &CryptoHash {
        match self {
            Self::V0(tx) => &tx.block_hash,
            Self::V1(tx) => &tx.block_hash,
        }
    }
}

impl BorshSerialize for Transaction {
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        match self {
            Self::V0(tx) => tx.serialize(writer),
            Self::V1(tx) => {
                BorshSerialize::serialize(&1_u8, writer)?;
                tx.serialize(writer)
            }
        }
    }
}

impl BorshDeserialize for Transaction {
    /// Upstream's two-byte version sniff. V0 starts with the signer id's
    /// 4-byte little-endian length, whose second byte is 0 because an account
    /// id is at most 64 bytes. V1 starts with the tag `1`, followed by that
    /// same length, whose first byte is non-zero. So: second byte 0 -> V0;
    /// else first byte 1 -> V1; else an invalid version tag.
    fn deserialize_reader<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        let u1 = u8::deserialize_reader(reader)?;
        let u2 = u8::deserialize_reader(reader)?;

        if u2 == 0 {
            let prefix = [u1, u2];
            let mut reader = (&prefix[..]).chain(reader);
            return Ok(Self::V0(TransactionV0::deserialize_reader(&mut reader)?));
        }

        if u1 == 1 {
            let prefix = [u2];
            let mut reader = (&prefix[..]).chain(reader);
            return Ok(Self::V1(TransactionV1::deserialize_reader(&mut reader)?));
        }

        Err(Error::new(
            ErrorKind::InvalidData,
            format!("invalid transaction version tag: {u1}"),
        ))
    }
}

/// A transaction and its signature. Upstream also caches the transaction hash
/// and size (`#[borsh(skip)]`, filled by `init`); neither is on the wire and
/// this crate needs neither.
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct SignedTransaction {
    pub transaction: Transaction,
    pub signature: Signature,
}

impl SignedTransaction {
    #[must_use]
    pub fn new(signature: Signature, transaction: Transaction) -> Self {
        Self {
            transaction,
            signature,
        }
    }
}
