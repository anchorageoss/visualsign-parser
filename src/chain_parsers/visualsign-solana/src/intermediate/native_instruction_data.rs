//! `parsed_instruction_data` for native-program instructions that Solana's
//! `jsonParsed` decoder covers, built from that decode instead of an IDL.
//!
//! The RPC returns inner (CPI) instructions of these programs parsed, with no
//! instruction data, so a consumer that identifies calls by discriminator has
//! nothing to read. This module recovers the discriminator from the decode's
//! `type` and carries its `info` as the call args.
//!
//! Discriminators are the exact bytes a consumer keys calls by: the variant
//! index at the program's native width (1 byte for SPL Token, Token-2022 and
//! ATA; `u32` LE for System), plus the sub-instruction byte for Token-2022
//! extension instructions. An instruction type not listed here gets no
//! `parsed_instruction_data`, so it stays unidentified rather than guessed.

use serde_json::Value;

use super::SolanaParsedInstructionDataIo;

const SYSTEM_PROGRAM: &str = "11111111111111111111111111111111";
const ATA_PROGRAM: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";
const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const TOKEN_2022_PROGRAM: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";

/// Token-2022 `PausableExtension` instruction prefix; its sub-instruction follows.
const TOKEN_2022_PAUSABLE_EXTENSION: u8 = 44;

/// `jsonParsed` type names as `solana-transaction-status` emits them, mapped to
/// the `TokenInstruction` variant index. Shared by SPL Token and Token-2022.
fn token_instruction(instruction_type: &str) -> Option<u8> {
    Some(match instruction_type {
        "initializeMint" => 0,
        "initializeAccount" => 1,
        "initializeMultisig" => 2,
        "transfer" => 3,
        "approve" => 4,
        "revoke" => 5,
        "setAuthority" => 6,
        "mintTo" => 7,
        "burn" => 8,
        "closeAccount" => 9,
        "freezeAccount" => 10,
        "thawAccount" => 11,
        "transferChecked" => 12,
        "approveChecked" => 13,
        "mintToChecked" => 14,
        "burnChecked" => 15,
        "initializeAccount2" => 16,
        "syncNative" => 17,
        "initializeAccount3" => 18,
        "initializeMultisig2" => 19,
        "initializeMint2" => 20,
        "getAccountDataSize" => 21,
        "initializeImmutableOwner" => 22,
        "amountToUiAmount" => 23,
        "uiAmountToAmount" => 24,
        _ => return None,
    })
}

/// Token-2022-only instructions: single-byte ones, then the pausable
/// extension's `[44, sub]`. Other extensions are not mapped.
fn token_2022_only_instruction(instruction_type: &str) -> Option<Vec<u8>> {
    let single = match instruction_type {
        "initializeMintCloseAuthority" => 25,
        "reallocate" => 29,
        "createNativeMint" => 31,
        "initializeNonTransferableMint" => 32,
        "initializePermanentDelegate" => 35,
        "withdrawExcessLamports" => 38,
        _ => {
            let sub = match instruction_type {
                "initializePausableConfig" => 0,
                "pause" => 1,
                "resume" => 2,
                _ => return None,
            };
            return Some(vec![TOKEN_2022_PAUSABLE_EXTENSION, sub]);
        }
    };
    Some(vec![single])
}

fn system_instruction(instruction_type: &str) -> Option<u32> {
    Some(match instruction_type {
        "createAccount" => 0,
        "assign" => 1,
        "transfer" => 2,
        "createAccountWithSeed" => 3,
        "advanceNonce" => 4,
        "withdrawFromNonce" => 5,
        "initializeNonce" => 6,
        "authorizeNonce" => 7,
        "allocate" => 8,
        "allocateWithSeed" => 9,
        "assignWithSeed" => 10,
        "transferWithSeed" => 11,
        "upgradeNonce" => 12,
        _ => return None,
    })
}

fn associated_token_instruction(instruction_type: &str) -> Option<u8> {
    Some(match instruction_type {
        "create" => 0,
        "createIdempotent" => 1,
        "recoverNested" => 2,
        _ => return None,
    })
}

/// The discriminator bytes for a `jsonParsed` instruction type, or `None` when
/// the program or type is not mapped.
fn discriminator(program_id: &str, instruction_type: &str) -> Option<Vec<u8>> {
    match program_id {
        SYSTEM_PROGRAM => {
            system_instruction(instruction_type).map(|index| index.to_le_bytes().to_vec())
        }
        ATA_PROGRAM => associated_token_instruction(instruction_type).map(|index| vec![index]),
        TOKEN_PROGRAM => token_instruction(instruction_type).map(|index| vec![index]),
        TOKEN_2022_PROGRAM => token_instruction(instruction_type)
            .map(|index| vec![index])
            .or_else(|| token_2022_only_instruction(instruction_type)),
        _ => None,
    }
}

/// Builds `parsed_instruction_data` from a canonicalized `jsonParsed` decode
/// (`{"type": .., "info": ..}`) of an instruction of `program_id`.
///
/// `raw_data` is the instruction data when known (top-level instructions). The
/// discriminator must then prefix it; on a mismatch this returns `None` and the
/// consumer keeps reading the raw bytes. Inner instructions pass `None`: the RPC
/// dropped their data, so the decode's `type` is all there is.
///
/// `info` goes into `program_call_args_json` whole; `named_accounts` stays
/// empty, as the decode names accounts under per-program keys, not IDL names.
pub(super) fn from_json_parsed(
    program_id: &str,
    parsed: &Value,
    raw_data: Option<&[u8]>,
) -> Option<SolanaParsedInstructionDataIo> {
    let instruction_type = parsed.get("type")?.as_str()?;
    let discriminator = discriminator(program_id, instruction_type)?;
    if let Some(data) = raw_data
        && !data.starts_with(&discriminator)
    {
        tracing::warn!(
            %program_id,
            %instruction_type,
            "jsonParsed type does not match the instruction data's discriminator"
        );
        return None;
    }
    let program_call_args_json = match parsed.get("info") {
        Some(info @ Value::Object(_)) => info.to_string(),
        _ => "{}".to_string(),
    };
    Some(SolanaParsedInstructionDataIo {
        instruction_name: instruction_type.to_string(),
        discriminator: hex::encode(discriminator),
        named_accounts: Default::default(),
        program_call_args_json,
        idl_source: String::new(),
        idl_hash: String::new(),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn discriminators_match_the_consumer_keys() {
        let cases: &[(&str, &str, &str)] = &[
            (TOKEN_PROGRAM, "transfer", "03"),
            (TOKEN_PROGRAM, "mintTo", "07"),
            (TOKEN_PROGRAM, "transferChecked", "0c"),
            (TOKEN_PROGRAM, "initializeImmutableOwner", "16"),
            (TOKEN_2022_PROGRAM, "transferChecked", "0c"),
            (TOKEN_2022_PROGRAM, "pause", "2c01"),
            (TOKEN_2022_PROGRAM, "resume", "2c02"),
            (SYSTEM_PROGRAM, "transfer", "02000000"),
            (SYSTEM_PROGRAM, "advanceNonce", "04000000"),
            (ATA_PROGRAM, "createIdempotent", "01"),
        ];
        for (program_id, instruction_type, expected) in cases {
            let parsed = json!({ "type": instruction_type, "info": {} });
            let io = from_json_parsed(program_id, &parsed, None)
                .unwrap_or_else(|| panic!("{instruction_type} on {program_id} is mapped"));
            assert_eq!(io.discriminator, *expected, "{instruction_type}");
            assert_eq!(io.instruction_name, *instruction_type);
        }
    }

    #[test]
    fn token_2022_only_types_are_not_mapped_for_spl_token() {
        let parsed = json!({ "type": "pause", "info": {} });
        assert!(from_json_parsed(TOKEN_PROGRAM, &parsed, None).is_none());
        assert!(from_json_parsed(TOKEN_2022_PROGRAM, &parsed, None).is_some());
    }

    #[test]
    fn unmapped_types_programs_and_shapes_get_nothing() {
        let unknown_type = json!({ "type": "transferCheckedWithFee", "info": {} });
        assert!(from_json_parsed(TOKEN_2022_PROGRAM, &unknown_type, None).is_none());

        let stake = json!({ "type": "delegate", "info": {} });
        assert!(
            from_json_parsed("Stake11111111111111111111111111111111111111", &stake, None).is_none()
        );

        // SPL Memo decodes to a bare string.
        let memo = json!("hello");
        assert!(
            from_json_parsed("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr", &memo, None).is_none()
        );
    }

    #[test]
    fn info_becomes_call_args() {
        let parsed = super::super::canonicalize_value(&json!({
            "type": "transferChecked",
            "info": {
                "tokenAmount": { "uiAmountString": "0.02", "amount": "20000", "decimals": 6 },
                "source": "S",
                "authority": "A",
            },
        }));
        let io = from_json_parsed(TOKEN_PROGRAM, &parsed, None).expect("mapped");
        assert_eq!(
            io.program_call_args_json,
            r#"{"authority":"A","source":"S","tokenAmount":{"amount":"20000","decimals":6,"uiAmountString":"0.02"}}"#
        );
        assert!(io.named_accounts.is_empty());
        assert!(io.idl_source.is_empty());
        assert!(io.idl_hash.is_empty());
    }

    #[test]
    fn raw_data_must_start_with_the_discriminator() {
        let parsed = json!({ "type": "transferChecked", "info": {} });
        let mut data = vec![12];
        data.extend_from_slice(&20_000u64.to_le_bytes());
        data.push(6);
        assert!(from_json_parsed(TOKEN_PROGRAM, &parsed, Some(&data)).is_some());

        data[0] = 3;
        assert!(from_json_parsed(TOKEN_PROGRAM, &parsed, Some(&data)).is_none());

        // Legacy ATA `create` carries no data; its `00` cannot prefix it.
        let create = json!({ "type": "create", "info": {} });
        assert!(from_json_parsed(ATA_PROGRAM, &create, Some(&[])).is_none());
    }
}
