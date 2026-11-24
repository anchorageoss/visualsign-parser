//! Anchor IDL account addressing, IDL account decoding, and the BPF
//! upgradeable loader's ProgramData parsing.
//!
//! The hand-written base58 / ed25519 on-curve / PDA / `create_with_seed`
//! math of the Python script this crate replaces is all delegated to
//! `solana_pubkey` here.
//!
//! The IDL account is found the way Anchor finds it:
//!
//! ```text
//! base    = find_program_address([], program_id)
//! address = create_with_seed(base, "anchor:idl", program_id)
//! ```

use anyhow::{Context as _, Result, bail};
use flate2::read::ZlibDecoder;
use solana_pubkey::Pubkey;
use std::io::Read as _;
use std::str::FromStr;

use crate::json::{self, Json};
use crate::rpc::Rpc;

/// Seed of the IDL account: `create_with_seed(base, "anchor:idl", program)`.
pub const IDL_SEED: &str = "anchor:idl";

/// Owner program whose accounts hold upgradeable Solana programs, and the
/// tags its accounts carry: 2 = "Program" (36 bytes: tag + ProgramData
/// address), 3 = "ProgramData" (tag, u64 LE last-deploy slot,
/// Option<Pubkey> authority).
pub const BPF_UPGRADEABLE_LOADER: &str = "BPFLoaderUpgradeab1e11111111111111111111111";
pub const PROGRAM_TAG: u32 = 2;
pub const PROGRAM_DATA_TAG: u32 = 3;

/// The RPC dataSlice that fetches a ProgramData account: the first 45 bytes
/// carry the tag, the last-deployment slot and the authority Option, so the
/// program binary that follows is never fetched.
pub const PROGRAM_DATA_SLICE: (u64, u64) = (0, 45);

/// IDL account layout: 8-byte discriminator, 32-byte authority, u32 LE
/// length, then that many bytes of zlib-compressed IDL JSON.
pub const HEADER_LEN: usize = 8 + 32 + 4;

/// Parse a base58 Solana program id.
pub fn parse_program_id(text: &str) -> Result<Pubkey> {
    Pubkey::from_str(text).map_err(|e| anyhow::anyhow!("invalid base58 string {text:?}: {e}"))
}

/// Address of the Anchor IDL account a program publishes on chain.
pub fn idl_address(program: &Pubkey) -> Result<Pubkey> {
    let (base, _bump) = Pubkey::try_find_program_address(&[], program)
        .context("no viable bump seed while deriving the IDL account")?;
    Pubkey::create_with_seed(&base, IDL_SEED, program)
        .context("failed to derive the IDL account with seed \"anchor:idl\"")
}

/// Decode the zlib-compressed IDL JSON out of an IDL account's bytes.
pub fn decode_idl_account(data: &[u8]) -> Result<Json> {
    if data.len() < HEADER_LEN {
        bail!(
            "IDL account is {} bytes, shorter than its {}-byte header",
            data.len(),
            HEADER_LEN
        );
    }
    let length =
        u32::from_le_bytes(data[40..44].try_into().context("short IDL length prefix")?) as usize;
    let Some(compressed) = data.get(HEADER_LEN..HEADER_LEN + length) else {
        bail!(
            "IDL account declares {length} bytes but holds {}",
            data.len() - HEADER_LEN
        );
    };
    let mut decoder = ZlibDecoder::new(compressed);
    let mut text = String::new();
    decoder
        .read_to_string(&mut text)
        .context("IDL account payload is not valid zlib data")?;
    json::parse(&text).map_err(anyhow::Error::msg)
}

/// ProgramData account address an upgradeable program points at, or `None`.
///
/// A program owned by the upgradeable BPF loader holds a little-endian u32
/// tag 2 ("Program") followed by the 32-byte ProgramData address. Any other
/// owner or tag means the program is not upgradeable.
pub fn program_data_address(owner: &str, data: &[u8]) -> Option<[u8; 32]> {
    if owner != BPF_UPGRADEABLE_LOADER {
        return None;
    }
    if data.len() < 4 + 32 {
        return None;
    }
    if u32::from_le_bytes(data[..4].try_into().ok()?) != PROGRAM_TAG {
        return None;
    }
    data[4..36].try_into().ok()
}

/// Last-deployment slot from ProgramData account bytes, or `None`.
///
/// ProgramData holds a little-endian u32 tag 3 ("ProgramData"), a u64 LE
/// slot (the last deployment slot), then an Option<Pubkey> upgrade
/// authority. The RPC call slices the first 45 bytes so the program binary
/// that follows is never fetched. Any other tag (or truncated data) means
/// no slot.
pub fn deployed_slot_from_program_data(data: &[u8]) -> Option<u64> {
    if data.len() < 4 + 8 {
        return None;
    }
    if u32::from_le_bytes(data[..4].try_into().ok()?) != PROGRAM_DATA_TAG {
        return None;
    }
    Some(u64::from_le_bytes(data[4..12].try_into().ok()?))
}

/// Slot the program was last deployed at, or `None`.
///
/// `None` whenever the program is not upgradeable or the two RPC lookups
/// fail for any reason: the slot is context in a provenance record, never
/// a reason to fail a fetch.
pub fn program_last_deployed_slot(rpc: &dyn Rpc, program_id: &str) -> Option<u64> {
    let info = rpc.get_account_info(program_id, None).ok()?;
    let data = info.data?;
    let owner = info.owner?;
    let address_bytes = program_data_address(&owner, &data)?;
    let address = Pubkey::new_from_array(address_bytes).to_string();
    let program_data = rpc
        .get_account_info(&address, Some(PROGRAM_DATA_SLICE))
        .ok()?;
    deployed_slot_from_program_data(&program_data.data?)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::ZlibEncoder;
    use std::io::Write as _;

    fn idl_account_payload(idl_json_text: &str) -> Vec<u8> {
        // The layout a deployed Anchor IDL account has: 8-byte
        // discriminator, 32-byte authority, u32 LE length, zlib-compressed
        // IDL JSON.
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(idl_json_text.as_bytes()).unwrap();
        let compressed = encoder.finish().unwrap();
        let mut data = Vec::new();
        data.extend_from_slice(&[7u8; 8]);
        data.extend_from_slice(&[0u8; 32]);
        data.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
        data.extend_from_slice(&compressed);
        data
    }

    #[test]
    fn idl_addresses_match_mainnet() {
        // Known answers, verified against `anchor idl fetch` and mainnet.
        let cases = [
            (
                "T1TANpTeScyeqVzzgNViGDNrkQ6qHz9KrSBS4aNXvGT",
                "HrPcWAWGndcmzCwP2R4ps6sZ8EN439sCD5xR32UZ9Rz4",
            ),
            (
                "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4",
                "C88XWfp26heEmDkmfSzeXP7Fd7GQJ2j9dDTUsyiZbUTa",
            ),
            (
                "proVF4pMXVaYqmy4NjniPh4pqKNfMmsihgd4wdkCX3u",
                "8wXL8gQduvMr6pmzhJnbUqsnnegJnmnPiZVPzehLjoeT",
            ),
            (
                "MarBmsSgKXdrN1egZf5sqe1TMai9K1rChYNDJgjq7aD",
                "9jWC3EixD3D7ChMrrSRw3opnGHQ8YxZGJqGkzcup3tAn",
            ),
        ];
        for (program, expected) in cases {
            let idl = idl_address(&parse_program_id(program).unwrap()).unwrap();
            assert_eq!(idl.to_string(), expected, "IDL account of {program}");
        }
    }

    #[test]
    fn idl_account_round_trips() {
        let idl_text =
            r#"{"address":"11111111111111111111111111111111","instructions":[{"name":"noop"}]}"#;
        let data = idl_account_payload(idl_text);
        let decoded = decode_idl_account(&data).unwrap();
        assert_eq!(
            json::pretty(&decoded),
            "{\n  \"address\": \"11111111111111111111111111111111\",\n  \"instructions\": [\n    {\n      \"name\": \"noop\"\n    }\n  ]\n}"
        );
        // Trailing bytes past the declared length are ignored, exactly like
        // Python's zlib.decompress over the sliced payload.
        let mut padded = data.clone();
        padded.extend_from_slice(&[9u8; 3]);
        let _ = decode_idl_account(&padded).unwrap();
    }

    #[test]
    fn short_idl_account_is_rejected() {
        let err = decode_idl_account(&[0u8; 8]).unwrap_err().to_string();
        assert!(
            err.contains("IDL account is 8 bytes, shorter than its 44-byte header"),
            "{err}"
        );
    }

    #[test]
    fn declared_length_mismatch_is_rejected() {
        let mut data = idl_account_payload("{}");
        // Shrink the payload without touching the length prefix.
        data.truncate(HEADER_LEN + 2);
        let err = decode_idl_account(&data).unwrap_err().to_string();
        assert!(
            err.contains("IDL account declares") && err.contains("bytes but holds"),
            "{err}"
        );
    }

    #[test]
    fn corrupt_zlib_payload_is_rejected() {
        let mut data = vec![0u8; HEADER_LEN];
        data[40..44].copy_from_slice(&3u32.to_le_bytes());
        data.extend_from_slice(&[0xF0, 0x00, 0x01]);
        assert!(decode_idl_account(&data).is_err());
    }

    #[test]
    fn upgradeable_program_yields_program_data_address() {
        let mut address = [0u8; 32];
        for (index, byte) in address.iter_mut().enumerate() {
            *byte = index as u8;
        }
        let mut data = Vec::new();
        data.extend_from_slice(&PROGRAM_TAG.to_le_bytes());
        data.extend_from_slice(&address);
        assert_eq!(
            program_data_address(BPF_UPGRADEABLE_LOADER, &data),
            Some(address)
        );
    }

    #[test]
    fn other_owner_is_not_upgradeable() {
        let mut data = Vec::new();
        data.extend_from_slice(&PROGRAM_TAG.to_le_bytes());
        data.extend_from_slice(&[0u8; 32]);
        assert_eq!(
            program_data_address("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA", &data),
            None
        );
    }

    #[test]
    fn non_program_tag_is_rejected() {
        let mut data = Vec::new();
        data.extend_from_slice(&PROGRAM_DATA_TAG.to_le_bytes());
        data.extend_from_slice(&[0u8; 32]);
        assert_eq!(program_data_address(BPF_UPGRADEABLE_LOADER, &data), None);
    }

    #[test]
    fn short_program_account_is_rejected() {
        assert_eq!(
            program_data_address(BPF_UPGRADEABLE_LOADER, &[0u8; 8]),
            None
        );
    }

    fn program_data_account(slot: u64, has_authority: bool) -> Vec<u8> {
        let authority: Vec<u8> = if has_authority {
            let mut option = vec![1u8];
            option.extend(0..32u8);
            option
        } else {
            vec![0u8]
        };
        let mut data = Vec::new();
        data.extend_from_slice(&PROGRAM_DATA_TAG.to_le_bytes());
        data.extend_from_slice(&slot.to_le_bytes());
        data.extend_from_slice(&authority);
        data
    }

    #[test]
    fn program_data_yields_last_deployed_slot() {
        let slot = 293_000_123;
        assert_eq!(
            deployed_slot_from_program_data(&program_data_account(slot, true)),
            Some(slot)
        );
    }

    #[test]
    fn program_data_without_authority_still_yields_slot() {
        // The authority Option is after the slot; slicing 45 bytes may also
        // drop it, so a zero-authority or truncated tail must not matter.
        assert_eq!(
            deployed_slot_from_program_data(&program_data_account(7, false)),
            Some(7)
        );
    }

    #[test]
    fn program_data_wrong_tag_is_rejected() {
        let mut data = Vec::new();
        data.extend_from_slice(&PROGRAM_TAG.to_le_bytes());
        data.extend_from_slice(&99u64.to_le_bytes());
        assert_eq!(deployed_slot_from_program_data(&data), None);
    }

    #[test]
    fn program_data_short_is_rejected() {
        assert_eq!(deployed_slot_from_program_data(&[0u8; 4]), None);
    }
}
