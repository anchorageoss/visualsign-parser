//! Hand-written prost messages for exactly the Cosmos SDK types this crate
//! decodes, so the crate does not depend on `cosmrs` or the full SDK protos.
//! Field numbers follow the upstream `.proto` files named on each type.
//!
//! Every decode is checked by re-encoding: prost drops unknown fields, so a
//! message that does not re-encode to its input bytes carries something the
//! parser did not show.

use prost::Message;

/// `google.protobuf.Any`.
#[derive(Clone, PartialEq, Message)]
pub struct Any {
    #[prost(string, tag = "1")]
    pub type_url: String,
    #[prost(bytes = "vec", tag = "2")]
    pub value: Vec<u8>,
}

/// `google.protobuf.Timestamp`.
#[derive(Clone, PartialEq, Message)]
pub struct Timestamp {
    #[prost(int64, tag = "1")]
    pub seconds: i64,
    #[prost(int32, tag = "2")]
    pub nanos: i32,
}

/// cosmos/base/v1beta1/coin.proto `Coin`.
#[derive(Clone, PartialEq, Message)]
pub struct Coin {
    #[prost(string, tag = "1")]
    pub denom: String,
    #[prost(string, tag = "2")]
    pub amount: String,
}

/// cosmos/tx/v1beta1/tx.proto `SignDoc`.
#[derive(Clone, PartialEq, Message)]
pub struct SignDoc {
    #[prost(bytes = "vec", tag = "1")]
    pub body_bytes: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub auth_info_bytes: Vec<u8>,
    #[prost(string, tag = "3")]
    pub chain_id: String,
    #[prost(uint64, tag = "4")]
    pub account_number: u64,
}

/// cosmos/tx/v1beta1/tx.proto `TxBody` (v0.53+: `unordered`, `timeout_timestamp`).
#[derive(Clone, PartialEq, Message)]
pub struct TxBody {
    #[prost(message, repeated, tag = "1")]
    pub messages: Vec<Any>,
    #[prost(string, tag = "2")]
    pub memo: String,
    #[prost(uint64, tag = "3")]
    pub timeout_height: u64,
    #[prost(bool, tag = "4")]
    pub unordered: bool,
    #[prost(message, optional, tag = "5")]
    pub timeout_timestamp: Option<Timestamp>,
    #[prost(message, repeated, tag = "1023")]
    pub extension_options: Vec<Any>,
    #[prost(message, repeated, tag = "2047")]
    pub non_critical_extension_options: Vec<Any>,
}

/// cosmos/tx/v1beta1/tx.proto `AuthInfo`.
#[derive(Clone, PartialEq, Message)]
pub struct AuthInfo {
    #[prost(message, repeated, tag = "1")]
    pub signer_infos: Vec<SignerInfo>,
    #[prost(message, optional, tag = "2")]
    pub fee: Option<Fee>,
    #[prost(message, optional, tag = "3")]
    pub tip: Option<Tip>,
}

/// cosmos/tx/v1beta1/tx.proto `SignerInfo`.
#[derive(Clone, PartialEq, Message)]
pub struct SignerInfo {
    #[prost(message, optional, tag = "1")]
    pub public_key: Option<Any>,
    #[prost(message, optional, tag = "2")]
    pub mode_info: Option<ModeInfo>,
    #[prost(uint64, tag = "3")]
    pub sequence: u64,
}

/// cosmos/tx/v1beta1/tx.proto `ModeInfo`.
#[derive(Clone, PartialEq, Message)]
pub struct ModeInfo {
    #[prost(oneof = "mode_info::Sum", tags = "1, 2")]
    pub sum: Option<mode_info::Sum>,
}

/// `ModeInfo` oneof.
pub mod mode_info {
    /// `ModeInfo.sum`.
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Sum {
        #[prost(message, tag = "1")]
        Single(Single),
        #[prost(message, tag = "2")]
        Multi(Multi),
    }

    /// `ModeInfo.Single`.
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Single {
        #[prost(int32, tag = "1")]
        pub mode: i32,
    }

    /// `ModeInfo.Multi`.
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Multi {
        #[prost(message, optional, tag = "1")]
        pub bitarray: Option<super::CompactBitArray>,
        #[prost(message, repeated, tag = "2")]
        pub mode_infos: Vec<super::ModeInfo>,
    }
}

/// cosmos/crypto/multisig/v1beta1/multisig.proto `CompactBitArray`.
#[derive(Clone, PartialEq, Message)]
pub struct CompactBitArray {
    #[prost(uint32, tag = "1")]
    pub extra_bits_stored: u32,
    #[prost(bytes = "vec", tag = "2")]
    pub elems: Vec<u8>,
}

/// cosmos/tx/v1beta1/tx.proto `Fee`.
#[derive(Clone, PartialEq, Message)]
pub struct Fee {
    #[prost(message, repeated, tag = "1")]
    pub amount: Vec<Coin>,
    #[prost(uint64, tag = "2")]
    pub gas_limit: u64,
    #[prost(string, tag = "3")]
    pub payer: String,
    #[prost(string, tag = "4")]
    pub granter: String,
}

/// cosmos/tx/v1beta1/tx.proto `Tip` (deprecated).
#[derive(Clone, PartialEq, Message)]
pub struct Tip {
    #[prost(message, repeated, tag = "1")]
    pub amount: Vec<Coin>,
    #[prost(string, tag = "2")]
    pub tipper: String,
}

/// cosmos/crypto/secp256k1/keys.proto `PubKey` (and other single-key types).
#[derive(Clone, PartialEq, Message)]
pub struct KeyBytes {
    #[prost(bytes = "vec", tag = "1")]
    pub key: Vec<u8>,
}

/// cosmos/crypto/multisig/keys.proto `LegacyAminoPubKey`.
#[derive(Clone, PartialEq, Message)]
pub struct LegacyAminoPubKey {
    #[prost(uint32, tag = "1")]
    pub threshold: u32,
    #[prost(message, repeated, tag = "2")]
    pub public_keys: Vec<Any>,
}

/// cosmos/bank/v1beta1/tx.proto `MsgSend`.
#[derive(Clone, PartialEq, Message)]
pub struct MsgSend {
    #[prost(string, tag = "1")]
    pub from_address: String,
    #[prost(string, tag = "2")]
    pub to_address: String,
    #[prost(message, repeated, tag = "3")]
    pub amount: Vec<Coin>,
}

/// cosmos/staking/v1beta1/tx.proto `MsgDelegate` / `MsgUndelegate`.
#[derive(Clone, PartialEq, Message)]
pub struct MsgDelegate {
    #[prost(string, tag = "1")]
    pub delegator_address: String,
    #[prost(string, tag = "2")]
    pub validator_address: String,
    #[prost(message, optional, tag = "3")]
    pub amount: Option<Coin>,
}

/// cosmos/staking/v1beta1/tx.proto `MsgBeginRedelegate`.
#[derive(Clone, PartialEq, Message)]
pub struct MsgBeginRedelegate {
    #[prost(string, tag = "1")]
    pub delegator_address: String,
    #[prost(string, tag = "2")]
    pub validator_src_address: String,
    #[prost(string, tag = "3")]
    pub validator_dst_address: String,
    #[prost(message, optional, tag = "4")]
    pub amount: Option<Coin>,
}

/// cosmos/distribution/v1beta1/tx.proto `MsgWithdrawDelegatorReward`.
#[derive(Clone, PartialEq, Message)]
pub struct MsgWithdrawDelegatorReward {
    #[prost(string, tag = "1")]
    pub delegator_address: String,
    #[prost(string, tag = "2")]
    pub validator_address: String,
}

/// cosmos/gov/v1beta1/tx.proto and cosmos/gov/v1/tx.proto `MsgVote`
/// (v1 adds `metadata = 4`).
#[derive(Clone, PartialEq, Message)]
pub struct MsgVote {
    #[prost(uint64, tag = "1")]
    pub proposal_id: u64,
    #[prost(string, tag = "2")]
    pub voter: String,
    #[prost(int32, tag = "3")]
    pub option: i32,
    #[prost(string, tag = "4")]
    pub metadata: String,
}

/// cosmos/authz/v1beta1/tx.proto `MsgExec`.
#[derive(Clone, PartialEq, Message)]
pub struct MsgExec {
    #[prost(string, tag = "1")]
    pub grantee: String,
    #[prost(message, repeated, tag = "2")]
    pub msgs: Vec<Any>,
}

/// ibc/core/client/v1/client.proto `Height`.
#[derive(Clone, PartialEq, Message)]
pub struct Height {
    #[prost(uint64, tag = "1")]
    pub revision_number: u64,
    #[prost(uint64, tag = "2")]
    pub revision_height: u64,
}

/// ibc/applications/transfer/v1/tx.proto `MsgTransfer` (fields 1 to 9; `token`
/// is deprecated in favour of `tokens` from ibc-go v9; v9's `forwarding` is not
/// modelled, so such a message is refused).
#[derive(Clone, PartialEq, Message)]
pub struct MsgTransfer {
    #[prost(string, tag = "1")]
    pub source_port: String,
    #[prost(string, tag = "2")]
    pub source_channel: String,
    #[prost(message, optional, tag = "3")]
    pub token: Option<Coin>,
    #[prost(string, tag = "4")]
    pub sender: String,
    #[prost(string, tag = "5")]
    pub receiver: String,
    #[prost(message, optional, tag = "6")]
    pub timeout_height: Option<Height>,
    #[prost(uint64, tag = "7")]
    pub timeout_timestamp: u64,
    #[prost(string, tag = "8")]
    pub memo: String,
    #[prost(message, repeated, tag = "9")]
    pub tokens: Vec<Coin>,
}

/// cosmwasm/wasm/v1/tx.proto `MsgExecuteContract`.
#[derive(Clone, PartialEq, Message)]
pub struct MsgExecuteContract {
    #[prost(string, tag = "1")]
    pub sender: String,
    #[prost(string, tag = "2")]
    pub contract: String,
    #[prost(bytes = "vec", tag = "3")]
    pub msg: Vec<u8>,
    #[prost(message, repeated, tag = "5")]
    pub funds: Vec<Coin>,
}

/// Decode `bytes` as `M` and require that it re-encodes to exactly `bytes`.
/// `Err(None)` if it does not parse, `Err(Some(m))` if it parses but is not
/// canonical (has fields the parser does not show, or a non-canonical encoding).
pub fn decode_exact<M: Message + Default>(bytes: &[u8]) -> Result<M, Option<M>> {
    let m = M::decode(bytes).map_err(|_| None)?;
    if m.encode_to_vec() == bytes {
        Ok(m)
    } else {
        Err(Some(m))
    }
}
