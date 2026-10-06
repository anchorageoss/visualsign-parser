//! `parsed_instruction_data` for native programs, which ship no IDL. One mapping
//! serves outer and inner instructions, so both yield identical output.

use std::collections::BTreeMap;

use borsh::BorshDeserialize;
use serde_json::{Value, json};
use solana_sdk::compute_budget::ComputeBudgetInstruction;
use solana_sdk::instruction::CompiledInstruction;
use solana_sdk::message::AccountKeys;
use solana_sdk::pubkey::Pubkey;

use super::{SolanaParsedInstructionDataIo, canonical_args_json, canonicalize_value};

pub(super) const SYSTEM_PROGRAM: &str = "11111111111111111111111111111111";
pub(super) const ADDRESS_LOOKUP_TABLE_PROGRAM: &str = "AddressLookupTab1e1111111111111111111111111";
pub(super) const ASSOCIATED_TOKEN_PROGRAM: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";
pub(super) const BPF_LOADER_2_PROGRAM: &str = "BPFLoader2111111111111111111111111111111111";
pub(super) const BPF_LOADER_UPGRADEABLE_PROGRAM: &str =
    "BPFLoaderUpgradeab1e11111111111111111111111";
pub(super) const COMPUTE_BUDGET_PROGRAM: &str = "ComputeBudget111111111111111111111111111111";
pub(super) const MEMO_V1_PROGRAM: &str = "Memo1UhkJRfHyvLMcVucJwxXeuD728EqVDDwQDxFMNo";
pub(super) const MEMO_PROGRAM: &str = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";
pub(super) const STAKE_PROGRAM: &str = "Stake11111111111111111111111111111111111111";
pub(super) const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
pub(super) const TOKEN_2022_PROGRAM: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";
pub(super) const VOTE_PROGRAM: &str = "Vote111111111111111111111111111111111111111";

/// `Native` programs Solana's `jsonParsed` decoder supports; a test keeps this
/// in step with the decoder.
pub(super) const SOLANA_JSON_PARSED_PROGRAMS: &[&str] = &[
    SYSTEM_PROGRAM,
    ADDRESS_LOOKUP_TABLE_PROGRAM,
    ASSOCIATED_TOKEN_PROGRAM,
    BPF_LOADER_2_PROGRAM,
    BPF_LOADER_UPGRADEABLE_PROGRAM,
    MEMO_V1_PROGRAM,
    MEMO_PROGRAM,
    STAKE_PROGRAM,
    TOKEN_PROGRAM,
    TOKEN_2022_PROGRAM,
    VOTE_PROGRAM,
];

/// Whether [`decode`] can decode instructions of `program_id` at all.
pub(super) fn is_decodable(program_id: &str) -> bool {
    program_id == COMPUTE_BUDGET_PROGRAM || SOLANA_JSON_PARSED_PROGRAMS.contains(&program_id)
}

/// Name the HSM-facing output uses for SPL Memo, which `jsonParsed` decodes
/// to a bare string with no instruction type.
const MEMO_INSTRUCTION_NAME: &str = "memo";

/// A `jsonParsed`-shaped decode of one instruction: the decoder's program
/// name and its canonicalized `parsed` value.
#[derive(Debug)]
pub(super) struct NativeDecode {
    /// The decoder's program name, e.g. `system`, `spl-token`.
    pub program: String,
    /// `{"info":..,"type":..}` for a typed instruction; a bare string for SPL
    /// Memo. Keys alphabetized at every level.
    pub parsed: Value,
}

/// Decodes a native instruction from its data and accounts. `account_keys`
/// must resolve every index in `instruction.accounts`.
pub(super) fn decode(
    program_id: &Pubkey,
    instruction: &CompiledInstruction,
    account_keys: &AccountKeys,
) -> Result<NativeDecode, String> {
    if *program_id == solana_sdk::compute_budget::id() {
        return Ok(NativeDecode {
            program: "compute-budget".to_string(),
            parsed: canonicalize_value(&decode_compute_budget(&instruction.data)?),
        });
    }
    let decoded = solana_transaction_status::parse_instruction::parse(
        program_id,
        instruction,
        account_keys,
        None,
    )
    .map_err(|e| e.to_string())?;
    Ok(NativeDecode {
        program: decoded.program,
        parsed: canonicalize_value(&decoded.parsed),
    })
}

/// Compute Budget in the `jsonParsed` shape, since Solana's decoder skips it.
fn decode_compute_budget(data: &[u8]) -> Result<Value, String> {
    let instruction = ComputeBudgetInstruction::try_from_slice(data)
        .map_err(|e| format!("Compute Budget instruction not parsable: {e}"))?;
    let (instruction_type, info) = match instruction {
        ComputeBudgetInstruction::Unused => {
            return Err("Compute Budget instruction 0 is deprecated".to_string());
        }
        ComputeBudgetInstruction::RequestHeapFrame(bytes) => {
            ("requestHeapFrame", json!({ "bytes": bytes }))
        }
        ComputeBudgetInstruction::SetComputeUnitLimit(units) => {
            ("setComputeUnitLimit", json!({ "units": units }))
        }
        ComputeBudgetInstruction::SetComputeUnitPrice(micro_lamports) => (
            "setComputeUnitPrice",
            json!({ "microLamports": micro_lamports }),
        ),
        ComputeBudgetInstruction::SetLoadedAccountsDataSizeLimit(bytes) => {
            ("setLoadedAccountsDataSizeLimit", json!({ "bytes": bytes }))
        }
    };
    Ok(json!({ "type": instruction_type, "info": info }))
}

/// Maps a `jsonParsed` decode to `parsed_instruction_data`. With `raw_data`
/// (outer, partially-decoded inner) the discriminator must prefix it.
/// `info` stays whole in the args: the HSM reads accounts from there.
pub(super) fn from_json_parsed(
    program_id: &str,
    parsed: &Value,
    raw_data: Option<&[u8]>,
) -> Result<SolanaParsedInstructionDataIo, String> {
    if program_id == MEMO_PROGRAM || program_id == MEMO_V1_PROGRAM {
        return from_memo(parsed);
    }
    let instruction_type = parsed
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "jsonParsed decode has no instruction type".to_string())?;
    let spec = find_spec(program_id, instruction_type).ok_or_else(|| {
        format!("jsonParsed instruction type {instruction_type} is not mapped for this program")
    })?;
    if let Some(data) = raw_data
        && !data_matches(spec, program_id, data)
    {
        return Err(format!(
            "instruction data does not start with the discriminator of {instruction_type}"
        ));
    }

    let args = match parsed.get("info") {
        Some(Value::Object(info)) => info.clone(),
        _ => serde_json::Map::new(),
    };
    let mut named_accounts = BTreeMap::new();
    for field in spec.accounts {
        match args.get(*field) {
            Some(Value::String(key)) => {
                named_accounts.insert((*field).to_string(), key.clone());
            }
            Some(Value::Array(keys)) if keys.iter().all(Value::is_string) => {
                for (i, key) in keys.iter().filter_map(Value::as_str).enumerate() {
                    named_accounts.insert(format!("{field}[{i}]"), key.to_string());
                }
            }
            // Absent, or `null` for an optional account that was not passed.
            _ => {}
        }
    }

    Ok(SolanaParsedInstructionDataIo {
        instruction_name: instruction_type.to_string(),
        discriminator: hex::encode(spec.discriminator),
        named_accounts,
        program_call_args_json: canonical_args_json(&args),
        idl_source: String::new(),
        idl_hash: String::new(),
    })
}

/// Memo has no discriminator: the data is the UTF-8 text, which `jsonParsed`
/// returns as a bare string with no accounts.
fn from_memo(parsed: &Value) -> Result<SolanaParsedInstructionDataIo, String> {
    let memo = parsed
        .as_str()
        .ok_or_else(|| "memo decode is not a string".to_string())?;
    let args = serde_json::Map::from_iter([("memo".to_string(), json!(memo))]);
    Ok(SolanaParsedInstructionDataIo {
        instruction_name: MEMO_INSTRUCTION_NAME.to_string(),
        discriminator: String::new(),
        named_accounts: BTreeMap::new(),
        program_call_args_json: canonical_args_json(&args),
        idl_source: String::new(),
        idl_hash: String::new(),
    })
}

/// ATA `create` is also accepted with empty data (the legacy encoding), and
/// the decoder reports both as `create`; it keeps the `00` discriminator.
fn data_matches(spec: &NativeInstructionSpec, program_id: &str, data: &[u8]) -> bool {
    data.starts_with(spec.discriminator)
        || (program_id == ASSOCIATED_TOKEN_PROGRAM && spec.name == "create" && data.is_empty())
}

/// One instruction of a native program, keyed by the `type` name Solana's
/// `jsonParsed` decoder emits for it.
struct NativeInstructionSpec {
    name: &'static str,
    /// The bytes the instruction data starts with.
    discriminator: &'static [u8],
    /// The `info` fields that hold account keys. Multisig instructions carry
    /// either the single authority field or the multisig one plus `signers`.
    accounts: &'static [&'static str],
}

const fn spec(
    name: &'static str,
    discriminator: &'static [u8],
    accounts: &'static [&'static str],
) -> NativeInstructionSpec {
    NativeInstructionSpec {
        name,
        discriminator,
        accounts,
    }
}

fn find_spec(program_id: &str, instruction_type: &str) -> Option<&'static NativeInstructionSpec> {
    let tables: &[&[NativeInstructionSpec]] = match program_id {
        SYSTEM_PROGRAM => &[SYSTEM],
        ADDRESS_LOOKUP_TABLE_PROGRAM => &[ADDRESS_LOOKUP_TABLE],
        ASSOCIATED_TOKEN_PROGRAM => &[ASSOCIATED_TOKEN],
        BPF_LOADER_2_PROGRAM => &[BPF_LOADER_2],
        BPF_LOADER_UPGRADEABLE_PROGRAM => &[BPF_LOADER_UPGRADEABLE],
        COMPUTE_BUDGET_PROGRAM => &[COMPUTE_BUDGET],
        STAKE_PROGRAM => &[STAKE],
        TOKEN_PROGRAM => &[TOKEN],
        TOKEN_2022_PROGRAM => &[TOKEN, TOKEN_2022_ONLY],
        VOTE_PROGRAM => &[VOTE],
        _ => return None,
    };
    tables
        .iter()
        .flat_map(|table| table.iter())
        .find(|spec| spec.name == instruction_type)
}

/// `SystemInstruction`, bincode: `u32` LE variant index.
const SYSTEM: &[NativeInstructionSpec] = &[
    spec("createAccount", &[0, 0, 0, 0], &["source", "newAccount"]),
    spec("assign", &[1, 0, 0, 0], &["account"]),
    spec("transfer", &[2, 0, 0, 0], &["source", "destination"]),
    spec(
        "createAccountWithSeed",
        &[3, 0, 0, 0],
        &["source", "newAccount"],
    ),
    spec(
        "advanceNonce",
        &[4, 0, 0, 0],
        &["nonceAccount", "recentBlockhashesSysvar", "nonceAuthority"],
    ),
    spec(
        "withdrawFromNonce",
        &[5, 0, 0, 0],
        &[
            "nonceAccount",
            "destination",
            "recentBlockhashesSysvar",
            "rentSysvar",
            "nonceAuthority",
        ],
    ),
    spec(
        "initializeNonce",
        &[6, 0, 0, 0],
        &["nonceAccount", "recentBlockhashesSysvar", "rentSysvar"],
    ),
    spec(
        "authorizeNonce",
        &[7, 0, 0, 0],
        &["nonceAccount", "nonceAuthority"],
    ),
    spec("allocate", &[8, 0, 0, 0], &["account"]),
    spec("allocateWithSeed", &[9, 0, 0, 0], &["account"]),
    spec("assignWithSeed", &[10, 0, 0, 0], &["account"]),
    spec(
        "transferWithSeed",
        &[11, 0, 0, 0],
        &["source", "sourceBase", "destination"],
    ),
    spec("upgradeNonce", &[12, 0, 0, 0], &["nonceAccount"]),
];

/// `AssociatedTokenAccountInstruction`, borsh: one byte.
const ASSOCIATED_TOKEN: &[NativeInstructionSpec] = &[
    spec(
        "create",
        &[0],
        &[
            "source",
            "account",
            "wallet",
            "mint",
            "systemProgram",
            "tokenProgram",
        ],
    ),
    spec(
        "createIdempotent",
        &[1],
        &[
            "source",
            "account",
            "wallet",
            "mint",
            "systemProgram",
            "tokenProgram",
        ],
    ),
    spec(
        "recoverNested",
        &[2],
        &[
            "nestedSource",
            "nestedMint",
            "destination",
            "nestedOwner",
            "ownerMint",
            "wallet",
            "tokenProgram",
        ],
    ),
];

/// Address Lookup Table `ProgramInstruction`, bincode: `u32` LE.
const ADDRESS_LOOKUP_TABLE: &[NativeInstructionSpec] = &[
    spec(
        "createLookupTable",
        &[0, 0, 0, 0],
        &[
            "lookupTableAccount",
            "lookupTableAuthority",
            "payerAccount",
            "systemProgram",
        ],
    ),
    spec(
        "freezeLookupTable",
        &[1, 0, 0, 0],
        &["lookupTableAccount", "lookupTableAuthority"],
    ),
    spec(
        "extendLookupTable",
        &[2, 0, 0, 0],
        &[
            "lookupTableAccount",
            "lookupTableAuthority",
            "payerAccount",
            "systemProgram",
        ],
    ),
    spec(
        "deactivateLookupTable",
        &[3, 0, 0, 0],
        &["lookupTableAccount", "lookupTableAuthority"],
    ),
    spec(
        "closeLookupTable",
        &[4, 0, 0, 0],
        &["lookupTableAccount", "lookupTableAuthority", "recipient"],
    ),
];

/// BPF Loader v2 `LoaderInstruction`, bincode: `u32` LE.
const BPF_LOADER_2: &[NativeInstructionSpec] = &[
    spec("write", &[0, 0, 0, 0], &["account"]),
    spec("finalize", &[1, 0, 0, 0], &["account"]),
];

/// `UpgradeableLoaderInstruction`, bincode: `u32` LE.
const BPF_LOADER_UPGRADEABLE: &[NativeInstructionSpec] = &[
    spec("initializeBuffer", &[0, 0, 0, 0], &["account", "authority"]),
    spec("write", &[1, 0, 0, 0], &["account", "authority"]),
    spec(
        "deployWithMaxDataLen",
        &[2, 0, 0, 0],
        &[
            "payerAccount",
            "programDataAccount",
            "programAccount",
            "bufferAccount",
            "rentSysvar",
            "clockSysvar",
            "systemProgram",
            "authority",
        ],
    ),
    spec(
        "upgrade",
        &[3, 0, 0, 0],
        &[
            "programDataAccount",
            "programAccount",
            "bufferAccount",
            "spillAccount",
            "rentSysvar",
            "clockSysvar",
            "authority",
        ],
    ),
    spec(
        "setAuthority",
        &[4, 0, 0, 0],
        &["account", "authority", "newAuthority"],
    ),
    spec(
        "close",
        &[5, 0, 0, 0],
        &["account", "recipient", "authority", "programAccount"],
    ),
    spec(
        "extendProgram",
        &[6, 0, 0, 0],
        &[
            "programDataAccount",
            "programAccount",
            "systemProgram",
            "payerAccount",
        ],
    ),
    spec(
        "setAuthorityChecked",
        &[7, 0, 0, 0],
        &["account", "authority", "newAuthority"],
    ),
    spec(
        "migrate",
        &[8, 0, 0, 0],
        &["programDataAccount", "programAccount", "authority"],
    ),
    spec(
        "extendProgramChecked",
        &[9, 0, 0, 0],
        &[
            "programDataAccount",
            "programAccount",
            "authority",
            "systemProgram",
            "payerAccount",
        ],
    ),
];

/// `ComputeBudgetInstruction`, borsh: one byte. Decoded by this crate, see
/// [`decode_compute_budget`]; variant 0 is deprecated and rejected there.
const COMPUTE_BUDGET: &[NativeInstructionSpec] = &[
    spec("requestHeapFrame", &[1], &[]),
    spec("setComputeUnitLimit", &[2], &[]),
    spec("setComputeUnitPrice", &[3], &[]),
    spec("setLoadedAccountsDataSizeLimit", &[4], &[]),
];

/// `StakeInstruction`, bincode: `u32` LE.
const STAKE: &[NativeInstructionSpec] = &[
    spec("initialize", &[0, 0, 0, 0], &["stakeAccount", "rentSysvar"]),
    spec(
        "authorize",
        &[1, 0, 0, 0],
        &["stakeAccount", "clockSysvar", "authority", "custodian"],
    ),
    spec(
        "delegate",
        &[2, 0, 0, 0],
        &[
            "stakeAccount",
            "voteAccount",
            "clockSysvar",
            "stakeHistorySysvar",
            "stakeConfigAccount",
            "stakeAuthority",
        ],
    ),
    spec(
        "split",
        &[3, 0, 0, 0],
        &["stakeAccount", "newSplitAccount", "stakeAuthority"],
    ),
    spec(
        "withdraw",
        &[4, 0, 0, 0],
        &[
            "stakeAccount",
            "destination",
            "clockSysvar",
            "stakeHistorySysvar",
            "withdrawAuthority",
            "custodian",
        ],
    ),
    spec(
        "deactivate",
        &[5, 0, 0, 0],
        &["stakeAccount", "clockSysvar", "stakeAuthority"],
    ),
    spec("setLockup", &[6, 0, 0, 0], &["stakeAccount", "custodian"]),
    spec(
        "merge",
        &[7, 0, 0, 0],
        &[
            "destination",
            "source",
            "clockSysvar",
            "stakeHistorySysvar",
            "stakeAuthority",
        ],
    ),
    spec(
        "authorizeWithSeed",
        &[8, 0, 0, 0],
        &["stakeAccount", "authorityBase", "clockSysvar", "custodian"],
    ),
    spec(
        "initializeChecked",
        &[9, 0, 0, 0],
        &["stakeAccount", "rentSysvar", "staker", "withdrawer"],
    ),
    spec(
        "authorizeChecked",
        &[10, 0, 0, 0],
        &[
            "stakeAccount",
            "clockSysvar",
            "authority",
            "newAuthority",
            "custodian",
        ],
    ),
    spec(
        "authorizeCheckedWithSeed",
        &[11, 0, 0, 0],
        &[
            "stakeAccount",
            "authorityBase",
            "clockSysvar",
            "newAuthorized",
            "custodian",
        ],
    ),
    spec(
        "setLockupChecked",
        &[12, 0, 0, 0],
        &["stakeAccount", "custodian"],
    ),
    spec("getMinimumDelegation", &[13, 0, 0, 0], &[]),
    spec(
        "deactivateDelinquent",
        &[14, 0, 0, 0],
        &["stakeAccount", "voteAccount", "referenceVoteAccount"],
    ),
    spec(
        "redelegate",
        &[15, 0, 0, 0],
        &[
            "stakeAccount",
            "newStakeAccount",
            "voteAccount",
            "stakeConfigAccount",
            "stakeAuthority",
        ],
    ),
    spec(
        "moveStake",
        &[16, 0, 0, 0],
        &["source", "destination", "stakeAuthority"],
    ),
    spec(
        "moveLamports",
        &[17, 0, 0, 0],
        &["source", "destination", "stakeAuthority"],
    ),
];

/// `VoteInstruction`, bincode: `u32` LE.
const VOTE: &[NativeInstructionSpec] = &[
    spec(
        "initialize",
        &[0, 0, 0, 0],
        &["voteAccount", "rentSysvar", "clockSysvar", "node"],
    ),
    spec(
        "authorize",
        &[1, 0, 0, 0],
        &["voteAccount", "clockSysvar", "authority"],
    ),
    spec(
        "vote",
        &[2, 0, 0, 0],
        &[
            "voteAccount",
            "slotHashesSysvar",
            "clockSysvar",
            "voteAuthority",
        ],
    ),
    spec(
        "withdraw",
        &[3, 0, 0, 0],
        &["voteAccount", "destination", "withdrawAuthority"],
    ),
    spec(
        "updateValidatorIdentity",
        &[4, 0, 0, 0],
        &["voteAccount", "newValidatorIdentity", "withdrawAuthority"],
    ),
    spec(
        "updateCommission",
        &[5, 0, 0, 0],
        &["voteAccount", "withdrawAuthority"],
    ),
    spec(
        "voteSwitch",
        &[6, 0, 0, 0],
        &[
            "voteAccount",
            "slotHashesSysvar",
            "clockSysvar",
            "voteAuthority",
        ],
    ),
    spec(
        "authorizeChecked",
        &[7, 0, 0, 0],
        &["voteAccount", "clockSysvar", "authority", "newAuthority"],
    ),
    spec(
        "updatevotestate",
        &[8, 0, 0, 0],
        &["voteAccount", "voteAuthority"],
    ),
    spec(
        "updatevotestateswitch",
        &[9, 0, 0, 0],
        &["voteAccount", "voteAuthority"],
    ),
    spec(
        "authorizeWithSeed",
        &[10, 0, 0, 0],
        &["voteAccount", "clockSysvar", "authorityBaseKey"],
    ),
    spec(
        "authorizeCheckedWithSeed",
        &[11, 0, 0, 0],
        &[
            "voteAccount",
            "clockSysvar",
            "authorityBaseKey",
            "newAuthority",
        ],
    ),
    spec(
        "compactupdatevotestate",
        &[12, 0, 0, 0],
        &["voteAccount", "voteAuthority"],
    ),
    spec(
        "compactupdatevotestateswitch",
        &[13, 0, 0, 0],
        &["voteAccount", "voteAuthority"],
    ),
    spec(
        "towersync",
        &[14, 0, 0, 0],
        &["voteAccount", "voteAuthority"],
    ),
    spec(
        "towersyncswitch",
        &[15, 0, 0, 0],
        &["voteAccount", "voteAuthority"],
    ),
];

/// `TokenInstruction`, one byte; shared by SPL Token and Token-2022.
const TOKEN: &[NativeInstructionSpec] = &[
    spec("initializeMint", &[0], &["mint", "rentSysvar"]),
    spec(
        "initializeAccount",
        &[1],
        &["account", "mint", "owner", "rentSysvar"],
    ),
    spec(
        "initializeMultisig",
        &[2],
        &["multisig", "rentSysvar", "signers"],
    ),
    spec(
        "transfer",
        &[3],
        &[
            "source",
            "destination",
            "authority",
            "multisigAuthority",
            "signers",
        ],
    ),
    spec(
        "approve",
        &[4],
        &["source", "delegate", "owner", "multisigOwner", "signers"],
    ),
    spec(
        "revoke",
        &[5],
        &["source", "owner", "multisigOwner", "signers"],
    ),
    spec(
        "setAuthority",
        &[6],
        &[
            "mint",
            "account",
            "authority",
            "multisigAuthority",
            "signers",
        ],
    ),
    spec(
        "mintTo",
        &[7],
        &[
            "mint",
            "account",
            "mintAuthority",
            "multisigMintAuthority",
            "signers",
        ],
    ),
    spec(
        "burn",
        &[8],
        &[
            "account",
            "mint",
            "authority",
            "multisigAuthority",
            "signers",
        ],
    ),
    spec(
        "closeAccount",
        &[9],
        &[
            "account",
            "destination",
            "owner",
            "multisigOwner",
            "signers",
        ],
    ),
    spec(
        "freezeAccount",
        &[10],
        &[
            "account",
            "mint",
            "freezeAuthority",
            "multisigFreezeAuthority",
            "signers",
        ],
    ),
    spec(
        "thawAccount",
        &[11],
        &[
            "account",
            "mint",
            "freezeAuthority",
            "multisigFreezeAuthority",
            "signers",
        ],
    ),
    spec(
        "transferChecked",
        &[12],
        &[
            "source",
            "mint",
            "destination",
            "authority",
            "multisigAuthority",
            "signers",
        ],
    ),
    spec(
        "approveChecked",
        &[13],
        &[
            "source",
            "mint",
            "delegate",
            "owner",
            "multisigOwner",
            "signers",
        ],
    ),
    spec(
        "mintToChecked",
        &[14],
        &[
            "mint",
            "account",
            "mintAuthority",
            "multisigMintAuthority",
            "signers",
        ],
    ),
    spec(
        "burnChecked",
        &[15],
        &[
            "account",
            "mint",
            "authority",
            "multisigAuthority",
            "signers",
        ],
    ),
    spec(
        "initializeAccount2",
        &[16],
        &["account", "mint", "rentSysvar"],
    ),
    spec("syncNative", &[17], &["account"]),
    spec("initializeAccount3", &[18], &["account", "mint"]),
    spec("initializeMultisig2", &[19], &["multisig", "signers"]),
    spec("initializeMint2", &[20], &["mint"]),
    spec("getAccountDataSize", &[21], &["mint"]),
    spec("initializeImmutableOwner", &[22], &["account"]),
    spec("amountToUiAmount", &[23], &["mint"]),
    spec("uiAmountToAmount", &[24], &["mint"]),
];

/// Token-2022 additions: single bytes, `[extension, sub]` pairs, and the 8-byte
/// `SplDiscriminate` hashes of the token group and metadata interfaces.
const TOKEN_2022_ONLY: &[NativeInstructionSpec] = &[
    spec("initializeMintCloseAuthority", &[25], &["mint"]),
    // TransferFeeExtension
    spec("initializeTransferFeeConfig", &[26, 0], &["mint"]),
    spec(
        "transferCheckedWithFee",
        &[26, 1],
        &[
            "source",
            "mint",
            "destination",
            "authority",
            "multisigAuthority",
            "signers",
        ],
    ),
    spec(
        "withdrawWithheldTokensFromMint",
        &[26, 2],
        &[
            "mint",
            "feeRecipient",
            "withdrawWithheldAuthority",
            "multisigWithdrawWithheldAuthority",
            "signers",
        ],
    ),
    spec(
        "withdrawWithheldTokensFromAccounts",
        &[26, 3],
        &[
            "mint",
            "feeRecipient",
            "sourceAccounts",
            "withdrawWithheldAuthority",
            "multisigWithdrawWithheldAuthority",
            "signers",
        ],
    ),
    spec(
        "harvestWithheldTokensToMint",
        &[26, 4],
        &["mint", "sourceAccounts"],
    ),
    spec(
        "setTransferFee",
        &[26, 5],
        &[
            "mint",
            "transferFeeConfigAuthority",
            "multisigtransferFeeConfigAuthority",
            "signers",
        ],
    ),
    // ConfidentialTransferExtension
    spec("initializeConfidentialTransferMint", &[27, 0], &["mint"]),
    spec(
        "updateConfidentialTransferMint",
        &[27, 1],
        &["mint", "confidentialTransferMintAuthority"],
    ),
    spec(
        "configureConfidentialTransferAccount",
        &[27, 2],
        &[
            "account",
            "mint",
            "proofContextStateAccount",
            "instructionsSysvar",
            "recordAccount",
            "owner",
            "multisigOwner",
            "signers",
        ],
    ),
    spec(
        "approveConfidentialTransferAccount",
        &[27, 3],
        &["account", "mint", "confidentialTransferAuditorAuthority"],
    ),
    spec(
        "emptyConfidentialTransferAccount",
        &[27, 4],
        &[
            "account",
            "proofContextStateAccount",
            "instructionsSysvar",
            "recordAccount",
            "owner",
            "multisigOwner",
            "signers",
        ],
    ),
    spec(
        "depositConfidentialTransfer",
        &[27, 5],
        &[
            "source",
            "destination",
            "mint",
            "owner",
            "multisigOwner",
            "signers",
        ],
    ),
    spec(
        "withdrawConfidentialTransfer",
        &[27, 6],
        &[
            "source",
            "destination",
            "mint",
            "instructionsSysvar",
            "equalityProofContextStateAccount",
            "equalityProofRecordAccount",
            "rangeProofContextStateAccount",
            "rangeProofRecordAccount",
            "owner",
            "multisigOwner",
            "signers",
        ],
    ),
    spec(
        "confidentialTransfer",
        &[27, 7],
        &[
            "source",
            "mint",
            "destination",
            "instructionsSysvar",
            "equalityProofContextStateAccount",
            "equalityProofRecordAccount",
            "ciphertextValidityProofContextStateAccount",
            "ciphertextValidityProofRecordAccount",
            "rangeProofContextStateAccount",
            "rangeProofRecordAccount",
            "owner",
            "multisigOwner",
            "signers",
        ],
    ),
    spec(
        "applyPendingConfidentialTransferBalance",
        &[27, 8],
        &["account", "owner", "multisigOwner", "signers"],
    ),
    spec(
        "enableConfidentialTransferConfidentialCredits",
        &[27, 9],
        &["account", "owner", "multisigOwner", "signers"],
    ),
    spec(
        "disableConfidentialTransferConfidentialCredits",
        &[27, 10],
        &["account", "owner", "multisigOwner", "signers"],
    ),
    spec(
        "enableConfidentialTransferNonConfidentialCredits",
        &[27, 11],
        &["account", "owner", "multisigOwner", "signers"],
    ),
    spec(
        "disableConfidentialTransferNonConfidentialCredits",
        &[27, 12],
        &["account", "owner", "multisigOwner", "signers"],
    ),
    spec(
        "confidentialTransferWithFee",
        &[27, 13],
        &[
            "source",
            "mint",
            "destination",
            "instructionsSysvar",
            "equalityProofContextStateAccount",
            "equalityProofRecordAccount",
            "transferAmountCiphertextValidityProofContextStateAccount",
            "transferAmountCiphertextValidityProofRecordAccount",
            "feeCiphertextValidityProofContextStateAccount",
            "feeCiphertextValidityProofRecordAccount",
            "feeSigmaProofContextStateAccount",
            "feeSigmaProofRecordAccount",
            "rangeProofContextStateAccount",
            "rangeProofRecordAccount",
            "owner",
            "multisigOwner",
            "signers",
        ],
    ),
    spec(
        "configureConfidentialAccountWithRegistry",
        &[27, 14],
        &["account", "mint", "registry"],
    ),
    // DefaultAccountStateExtension
    spec("initializeDefaultAccountState", &[28, 0], &["mint"]),
    spec(
        "updateDefaultAccountState",
        &[28, 1],
        &[
            "mint",
            "freezeAuthority",
            "multisigFreezeAuthority",
            "signers",
        ],
    ),
    spec(
        "reallocate",
        &[29],
        &[
            "account",
            "payer",
            "systemProgram",
            "owner",
            "multisigOwner",
            "signers",
        ],
    ),
    // MemoTransferExtension
    spec(
        "enableRequiredMemoTransfers",
        &[30, 0],
        &["account", "owner", "multisigOwner", "signers"],
    ),
    spec(
        "disableRequiredMemoTransfers",
        &[30, 1],
        &["account", "owner", "multisigOwner", "signers"],
    ),
    spec(
        "createNativeMint",
        &[31],
        &["payer", "nativeMint", "systemProgram"],
    ),
    spec("initializeNonTransferableMint", &[32], &["mint"]),
    // InterestBearingMintExtension
    spec("initializeInterestBearingConfig", &[33, 0], &["mint"]),
    spec(
        "updateInterestBearingConfigRate",
        &[33, 1],
        &["mint", "rateAuthority", "multisigRateAuthority", "signers"],
    ),
    // CpiGuardExtension
    spec(
        "enableCpiGuard",
        &[34, 0],
        &["account", "owner", "multisigOwner", "signers"],
    ),
    spec(
        "disableCpiGuard",
        &[34, 1],
        &["account", "owner", "multisigOwner", "signers"],
    ),
    spec("initializePermanentDelegate", &[35], &["mint"]),
    // TransferHookExtension
    spec("initializeTransferHook", &[36, 0], &["mint"]),
    spec(
        "updateTransferHook",
        &[36, 1],
        &["mint", "authority", "multisigAuthority", "signers"],
    ),
    // ConfidentialTransferFeeExtension
    spec(
        "initializeConfidentialTransferFeeConfig",
        &[37, 0],
        &["mint"],
    ),
    spec(
        "withdrawWithheldConfidentialTransferTokensFromMint",
        &[37, 1],
        &[
            "mint",
            "feeRecipient",
            "proofContextStateAccount",
            "instructionsSysvar",
            "recordAccount",
            "withdrawWithheldAuthority",
            "multisigWithdrawWithheldAuthority",
            "signers",
        ],
    ),
    spec(
        "withdrawWithheldConfidentialTransferTokensFromAccounts",
        &[37, 2],
        &[
            "mint",
            "feeRecipient",
            "proofContextStateAccount",
            "instructionsSysvar",
            "proofAccount",
            "sourceAccounts",
            "withdrawWithheldAuthority",
            "multisigWithdrawWithheldAuthority",
            "signers",
        ],
    ),
    spec(
        "harvestWithheldConfidentialTransferTokensToMint",
        &[37, 3],
        &["mint", "sourceAccounts"],
    ),
    spec(
        "enableConfidentialTransferFeeHarvestToMint",
        &[37, 4],
        &["account", "owner", "multisigOwner", "signers"],
    ),
    spec(
        "disableConfidentialTransferFeeHarvestToMint",
        &[37, 5],
        &["account", "owner", "multisigOwner", "signers"],
    ),
    spec(
        "withdrawExcessLamports",
        &[38],
        &[
            "source",
            "destination",
            "authority",
            "multisigAuthority",
            "signers",
        ],
    ),
    // MetadataPointerExtension
    spec("initializeMetadataPointer", &[39, 0], &["mint"]),
    spec(
        "updateMetadataPointer",
        &[39, 1],
        &["mint", "authority", "multisigAuthority", "signers"],
    ),
    // GroupPointerExtension
    spec("initializeGroupPointer", &[40, 0], &["mint"]),
    spec(
        "updateGroupPointer",
        &[40, 1],
        &["mint", "authority", "multisigAuthority", "signers"],
    ),
    // GroupMemberPointerExtension
    spec("initializeGroupMemberPointer", &[41, 0], &["mint"]),
    spec(
        "updateGroupMemberPointer",
        &[41, 1],
        &["mint", "authority", "multisigAuthority", "signers"],
    ),
    // ConfidentialMintBurnExtension
    spec("initializeConfidentialMintBurnMint", &[42, 0], &["mint"]),
    spec(
        "rotateConfidentialMintBurnSupplyElGamalPubkey",
        &[42, 1],
        &[
            "mint",
            "proofAccount",
            "instructionsSysvar",
            "owner",
            "multisigOwner",
            "signers",
        ],
    ),
    spec(
        "updateConfidentialMintBurnDecryptableSupply",
        &[42, 2],
        &["mint", "owner", "multisigOwner", "signers"],
    ),
    spec(
        "confidentialMint",
        &[42, 3],
        &[
            "destination",
            "mint",
            "instructionsSysvar",
            "equalityProofContextStateAccount",
            "equalityProofRecordAccount",
            "ciphertextValidityProofContextStateAccount",
            "ciphertextValidityProofRecordAccount",
            "rangeProofContextStateAccount",
            "rangeProofRecordAccount",
            "owner",
            "multisigOwner",
            "signers",
        ],
    ),
    spec(
        "confidentialBurn",
        &[42, 4],
        &[
            "destination",
            "mint",
            "instructionsSysvar",
            "equalityProofContextStateAccount",
            "equalityProofRecordAccount",
            "ciphertextValidityProofContextStateAccount",
            "ciphertextValidityProofRecordAccount",
            "rangeProofContextStateAccount",
            "rangeProofRecordAccount",
            "owner",
            "multisigOwner",
            "signers",
        ],
    ),
    spec(
        "applyPendingBurn",
        &[42, 5],
        &["mint", "owner", "multisigOwner", "signers"],
    ),
    // ScaledUiAmountExtension
    spec("initializeScaledUiAmountConfig", &[43, 0], &["mint"]),
    spec(
        "updateMultiplier",
        &[43, 1],
        &["mint", "authority", "multisigAuthority", "signers"],
    ),
    // PausableExtension
    spec("initializePausableConfig", &[44, 0], &["mint"]),
    spec(
        "pause",
        &[44, 1],
        &["mint", "authority", "multisigAuthority", "signers"],
    ),
    spec(
        "resume",
        &[44, 2],
        &["mint", "authority", "multisigAuthority", "signers"],
    ),
    // spl_token_group_interface: sha256("spl_token_group_interface:<name>")[..8]
    spec(
        "initializeTokenGroup",
        &[0x79, 0x71, 0x6c, 0x27, 0x36, 0x33, 0x00, 0x04],
        &["group", "mint", "mintAuthority"],
    ),
    spec(
        "updateTokenGroupMaxSize",
        &[0x6c, 0x25, 0xab, 0x8f, 0xf8, 0x1e, 0x12, 0x6e],
        &["group", "updateAuthority"],
    ),
    spec(
        "updateTokenGroupAuthority",
        &[0xa1, 0x69, 0x58, 0x01, 0xed, 0xdd, 0xd8, 0xcb],
        &["group", "updateAuthority"],
    ),
    spec(
        "initializeTokenGroupMember",
        &[0x98, 0x20, 0xde, 0xb0, 0xdf, 0xed, 0x74, 0x86],
        &[
            "member",
            "memberMint",
            "memberMintAuthority",
            "group",
            "groupUpdateAuthority",
        ],
    ),
    // spl_token_metadata_interface: sha256("spl_token_metadata_interface:<name>")[..8]
    spec(
        "initializeTokenMetadata",
        &[0xd2, 0xe1, 0x1e, 0xa2, 0x58, 0xb8, 0x4d, 0x8d],
        &["metadata", "updateAuthority", "mint", "mintAuthority"],
    ),
    spec(
        "updateTokenMetadataField",
        &[0xdd, 0xe9, 0x31, 0x2d, 0xb5, 0xca, 0xdc, 0xc8],
        &["metadata", "updateAuthority"],
    ),
    spec(
        "removeTokenMetadataKey",
        &[0xea, 0x12, 0x20, 0x38, 0x59, 0x8d, 0x25, 0xb5],
        &["metadata", "updateAuthority"],
    ),
    spec(
        "updateTokenMetadataAuthority",
        &[0xd7, 0xe4, 0xa6, 0xe4, 0x54, 0x64, 0x56, 0x7b],
        &["metadata", "updateAuthority"],
    ),
    spec(
        "emitTokenMetadata",
        &[0xfa, 0xa6, 0xb4, 0xfa, 0x0d, 0x0c, 0xb8, 0x46],
        &["metadata"],
    ),
];

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::str::FromStr;

    use serde_json::json;

    use super::*;

    /// The real decoder on a self-contained instruction: account `i` is key
    /// `i`, the program is the key after the last account.
    fn decode_with(program_id: &str, accounts: &[Pubkey], data: &[u8]) -> NativeDecode {
        let program_id = Pubkey::from_str(program_id).unwrap();
        let mut keys = accounts.to_vec();
        keys.push(program_id);
        let instruction = CompiledInstruction {
            program_id_index: accounts.len() as u8,
            accounts: (0..accounts.len() as u8).collect(),
            data: data.to_vec(),
        };
        decode(&program_id, &instruction, &AccountKeys::new(&keys, None)).expect("decodes")
    }

    #[test]
    fn discriminators_match_the_consumer_keys() {
        let cases: &[(&str, &str, &str)] = &[
            (TOKEN_PROGRAM, "transfer", "03"),
            (TOKEN_PROGRAM, "mintTo", "07"),
            (TOKEN_PROGRAM, "transferChecked", "0c"),
            (TOKEN_PROGRAM, "initializeImmutableOwner", "16"),
            (TOKEN_2022_PROGRAM, "transferChecked", "0c"),
            (TOKEN_2022_PROGRAM, "transferCheckedWithFee", "1a01"),
            (TOKEN_2022_PROGRAM, "pause", "2c01"),
            (TOKEN_2022_PROGRAM, "resume", "2c02"),
            (
                TOKEN_2022_PROGRAM,
                "updateTokenMetadataField",
                "dde9312db5cadcc8",
            ),
            (SYSTEM_PROGRAM, "transfer", "02000000"),
            (SYSTEM_PROGRAM, "advanceNonce", "04000000"),
            (ASSOCIATED_TOKEN_PROGRAM, "create", "00"),
            (ASSOCIATED_TOKEN_PROGRAM, "createIdempotent", "01"),
            (STAKE_PROGRAM, "delegate", "02000000"),
            (STAKE_PROGRAM, "withdraw", "04000000"),
            (VOTE_PROGRAM, "withdraw", "03000000"),
            (
                ADDRESS_LOOKUP_TABLE_PROGRAM,
                "extendLookupTable",
                "02000000",
            ),
            (BPF_LOADER_2_PROGRAM, "finalize", "01000000"),
            (BPF_LOADER_UPGRADEABLE_PROGRAM, "upgrade", "03000000"),
            (COMPUTE_BUDGET_PROGRAM, "setComputeUnitPrice", "03"),
        ];
        for (program_id, instruction_type, expected) in cases {
            let parsed = json!({ "type": instruction_type, "info": {} });
            let io = from_json_parsed(program_id, &parsed, None)
                .unwrap_or_else(|e| panic!("{instruction_type} on {program_id} is mapped: {e}"));
            assert_eq!(io.discriminator, *expected, "{instruction_type}");
            assert_eq!(io.instruction_name, *instruction_type);
            assert!(io.idl_source.is_empty());
            assert!(io.idl_hash.is_empty());
        }
    }

    /// The 8-byte interface discriminators are `sha256(hash_input)[..8]`.
    #[test]
    fn interface_discriminators_are_the_hash_prefixes() {
        for (instruction_type, hash_input) in [
            (
                "initializeTokenGroup",
                "spl_token_group_interface:initialize_token_group",
            ),
            (
                "updateTokenGroupMaxSize",
                "spl_token_group_interface:update_group_max_size",
            ),
            (
                "updateTokenGroupAuthority",
                "spl_token_group_interface:update_authority",
            ),
            (
                "initializeTokenGroupMember",
                "spl_token_group_interface:initialize_member",
            ),
            (
                "initializeTokenMetadata",
                "spl_token_metadata_interface:initialize_account",
            ),
            (
                "updateTokenMetadataField",
                "spl_token_metadata_interface:updating_field",
            ),
            (
                "removeTokenMetadataKey",
                "spl_token_metadata_interface:remove_key_ix",
            ),
            (
                "updateTokenMetadataAuthority",
                "spl_token_metadata_interface:update_the_authority",
            ),
            ("emitTokenMetadata", "spl_token_metadata_interface:emitter"),
        ] {
            let spec = find_spec(TOKEN_2022_PROGRAM, instruction_type).expect("mapped");
            let hash = solana_sdk::hash::hash(hash_input.as_bytes()).to_bytes();
            assert_eq!(spec.discriminator, &hash[..8], "{instruction_type}");
        }
    }

    #[test]
    fn token_2022_only_types_are_not_mapped_for_spl_token() {
        let parsed = json!({ "type": "pause", "info": {} });
        assert!(from_json_parsed(TOKEN_PROGRAM, &parsed, None).is_err());
        assert!(from_json_parsed(TOKEN_2022_PROGRAM, &parsed, None).is_ok());
    }

    #[test]
    fn unmapped_types_programs_and_shapes_are_errors() {
        let unknown_type = json!({ "type": "notAnInstruction", "info": {} });
        assert_eq!(
            from_json_parsed(TOKEN_2022_PROGRAM, &unknown_type, None).unwrap_err(),
            "jsonParsed instruction type notAnInstruction is not mapped for this program"
        );

        let stake_pool = "SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy";
        let deposit = json!({ "type": "depositSol", "info": {} });
        assert!(from_json_parsed(stake_pool, &deposit, None).is_err());

        let untyped = json!({ "info": {} });
        assert_eq!(
            from_json_parsed(SYSTEM_PROGRAM, &untyped, None).unwrap_err(),
            "jsonParsed decode has no instruction type"
        );
    }

    #[test]
    fn account_fields_are_named_and_stay_in_the_args() {
        let parsed = canonicalize_value(&json!({
            "type": "transferChecked",
            "info": {
                "tokenAmount": { "uiAmountString": "0.02", "amount": "20000", "decimals": 6 },
                "source": "S",
                "mint": "M",
                "destination": "D",
                "multisigAuthority": "A",
                "signers": ["K1", "K2"],
            },
        }));
        let io = from_json_parsed(TOKEN_PROGRAM, &parsed, None).expect("mapped");
        assert_eq!(
            io.named_accounts,
            BTreeMap::from([
                ("destination".to_string(), "D".to_string()),
                ("mint".to_string(), "M".to_string()),
                ("multisigAuthority".to_string(), "A".to_string()),
                ("signers[0]".to_string(), "K1".to_string()),
                ("signers[1]".to_string(), "K2".to_string()),
                ("source".to_string(), "S".to_string()),
            ])
        );
        assert_eq!(
            io.program_call_args_json,
            r#"{"destination":"D","mint":"M","multisigAuthority":"A","signers":["K1","K2"],"source":"S","tokenAmount":{"amount":"20000","decimals":6,"uiAmountString":"0.02"}}"#
        );
    }

    /// A pubkey-valued arg (the new owner, read from the data) is not named,
    /// nor is an optional account passed as `null`.
    #[test]
    fn pubkey_args_and_null_accounts_are_not_named() {
        let parsed = json!({
            "type": "initializeAccount3",
            "info": { "account": "A", "mint": "M", "owner": "O" },
        });
        let io = from_json_parsed(TOKEN_PROGRAM, &parsed, None).expect("mapped");
        assert_eq!(io.named_accounts.len(), 2);
        assert!(!io.named_accounts.contains_key("owner"));
        assert_eq!(
            io.program_call_args_json,
            r#"{"account":"A","mint":"M","owner":"O"}"#
        );

        let parsed = json!({
            "type": "setAuthority",
            "info": { "account": "A", "authority": "X", "newAuthority": null },
        });
        let io = from_json_parsed(BPF_LOADER_UPGRADEABLE_PROGRAM, &parsed, None).expect("mapped");
        assert_eq!(io.named_accounts.len(), 2);
        assert_eq!(
            io.program_call_args_json,
            r#"{"account":"A","authority":"X","newAuthority":null}"#
        );
    }

    #[test]
    fn raw_data_must_start_with_the_discriminator() {
        let parsed = json!({ "type": "transferChecked", "info": {} });
        let mut data = vec![12];
        data.extend_from_slice(&20_000u64.to_le_bytes());
        data.push(6);
        assert!(from_json_parsed(TOKEN_PROGRAM, &parsed, Some(&data)).is_ok());

        data[0] = 3;
        assert_eq!(
            from_json_parsed(TOKEN_PROGRAM, &parsed, Some(&data)).unwrap_err(),
            "instruction data does not start with the discriminator of transferChecked"
        );

        // ATA `create` keeps `00` whether the data carries it or is empty.
        let create = json!({ "type": "create", "info": {} });
        for data in [&[][..], &[0][..]] {
            let io =
                from_json_parsed(ASSOCIATED_TOKEN_PROGRAM, &create, Some(data)).expect("create");
            assert_eq!(io.discriminator, "00");
        }
        let idempotent = json!({ "type": "createIdempotent", "info": {} });
        assert!(from_json_parsed(ASSOCIATED_TOKEN_PROGRAM, &idempotent, Some(&[])).is_err());
    }

    #[test]
    fn memo_has_no_discriminator_and_carries_the_text() {
        for program_id in [MEMO_PROGRAM, MEMO_V1_PROGRAM] {
            let io = from_json_parsed(program_id, &json!("hello"), Some(b"hello")).expect("memo");
            assert_eq!(io.instruction_name, "memo");
            assert_eq!(io.discriminator, "");
            assert!(io.named_accounts.is_empty());
            assert_eq!(io.program_call_args_json, r#"{"memo":"hello"}"#);
        }
        assert!(from_json_parsed(MEMO_PROGRAM, &json!({ "type": "memo" }), None).is_err());
    }

    #[test]
    fn compute_budget_is_decoded_by_this_crate() {
        let decoded = decode_with(
            COMPUTE_BUDGET_PROGRAM,
            &[],
            &[3, 0x88, 0x13, 0, 0, 0, 0, 0, 0],
        );
        assert_eq!(decoded.program, "compute-budget");
        assert_eq!(
            decoded.parsed,
            json!({ "info": { "microLamports": 5000 }, "type": "setComputeUnitPrice" })
        );
        let io = from_json_parsed(
            COMPUTE_BUDGET_PROGRAM,
            &decoded.parsed,
            Some(&[3, 0x88, 0x13, 0, 0, 0, 0, 0, 0]),
        )
        .expect("mapped");
        assert_eq!(io.instruction_name, "setComputeUnitPrice");
        assert_eq!(io.discriminator, "03");
        assert!(io.named_accounts.is_empty());
        assert_eq!(io.program_call_args_json, r#"{"microLamports":5000}"#);

        let decoded = decode_with(COMPUTE_BUDGET_PROGRAM, &[], &[2, 0x40, 0x0d, 0x03, 0x00]);
        assert_eq!(decoded.parsed["type"], "setComputeUnitLimit");
        assert_eq!(decoded.parsed["info"]["units"], 200_000);

        let program_id = Pubkey::from_str(COMPUTE_BUDGET_PROGRAM).unwrap();
        let deprecated = CompiledInstruction {
            program_id_index: 0,
            accounts: vec![],
            data: vec![0],
        };
        let keys = [program_id];
        assert_eq!(
            decode(&program_id, &deprecated, &AccountKeys::new(&keys, None)).unwrap_err(),
            "Compute Budget instruction 0 is deprecated"
        );
    }

    /// Real encodings of a sample across the bincode and borsh programs decode
    /// and then map to the same discriminator the data starts with.
    #[test]
    fn decoded_instructions_map_to_their_own_discriminator() {
        let keys: [Pubkey; 6] = std::array::from_fn(|_| Pubkey::new_unique());
        let mut stake_withdraw = vec![4, 0, 0, 0];
        stake_withdraw.extend_from_slice(&7u64.to_le_bytes());
        let mut vote_withdraw = vec![3, 0, 0, 0];
        vote_withdraw.extend_from_slice(&9u64.to_le_bytes());
        let mut system_transfer = vec![2, 0, 0, 0];
        system_transfer.extend_from_slice(&1001u64.to_le_bytes());
        struct Case<'a> {
            program_id: &'a str,
            num_accounts: usize,
            data: &'a [u8],
            instruction_type: &'a str,
            accounts: &'a [&'a str],
        }
        let cases = [
            Case {
                program_id: SYSTEM_PROGRAM,
                num_accounts: 2,
                data: &system_transfer,
                instruction_type: "transfer",
                accounts: &["source", "destination"],
            },
            Case {
                program_id: STAKE_PROGRAM,
                num_accounts: 5,
                data: &stake_withdraw,
                instruction_type: "withdraw",
                accounts: &[
                    "stakeAccount",
                    "destination",
                    "clockSysvar",
                    "stakeHistorySysvar",
                    "withdrawAuthority",
                ],
            },
            Case {
                program_id: VOTE_PROGRAM,
                num_accounts: 3,
                data: &vote_withdraw,
                instruction_type: "withdraw",
                accounts: &["voteAccount", "destination", "withdrawAuthority"],
            },
            Case {
                program_id: ADDRESS_LOOKUP_TABLE_PROGRAM,
                num_accounts: 2,
                data: &[3, 0, 0, 0],
                instruction_type: "deactivateLookupTable",
                accounts: &["lookupTableAccount", "lookupTableAuthority"],
            },
            Case {
                program_id: TOKEN_2022_PROGRAM,
                num_accounts: 2,
                data: &[44, 1],
                instruction_type: "pause",
                accounts: &["mint", "authority"],
            },
            Case {
                program_id: ASSOCIATED_TOKEN_PROGRAM,
                num_accounts: 6,
                data: &[],
                instruction_type: "create",
                accounts: &[
                    "source",
                    "account",
                    "wallet",
                    "mint",
                    "systemProgram",
                    "tokenProgram",
                ],
            },
        ];
        for case in cases {
            let decoded = decode_with(case.program_id, &keys[..case.num_accounts], case.data);
            let io = from_json_parsed(case.program_id, &decoded.parsed, Some(case.data))
                .unwrap_or_else(|e| panic!("{}: {e}", case.instruction_type));
            assert_eq!(io.instruction_name, case.instruction_type);
            let named: Vec<&str> = case
                .accounts
                .iter()
                .map(|field| io.named_accounts[*field].as_str())
                .collect();
            let expected: Vec<String> = keys[..case.num_accounts]
                .iter()
                .map(Pubkey::to_string)
                .collect();
            assert_eq!(
                named, expected,
                "{} accounts in order",
                case.instruction_type
            );
        }
    }

    /// Every spec maps a type the decoder's own enum spells the same way, so a
    /// renamed or dropped type fails here rather than at decode time.
    #[test]
    fn every_program_table_is_internally_consistent() {
        let tables: &[(&str, &[NativeInstructionSpec])] = &[
            ("system", SYSTEM),
            ("ata", ASSOCIATED_TOKEN),
            ("alt", ADDRESS_LOOKUP_TABLE),
            ("loader2", BPF_LOADER_2),
            ("loader3", BPF_LOADER_UPGRADEABLE),
            ("compute-budget", COMPUTE_BUDGET),
            ("stake", STAKE),
            ("vote", VOTE),
            ("token", TOKEN),
            ("token-2022", TOKEN_2022_ONLY),
        ];
        for (label, table) in tables {
            let mut names: Vec<&str> = table.iter().map(|spec| spec.name).collect();
            names.sort_unstable();
            names.dedup();
            assert_eq!(names.len(), table.len(), "{label}: duplicate type name");
            let mut discriminators: Vec<&[u8]> =
                table.iter().map(|spec| spec.discriminator).collect();
            discriminators.sort_unstable();
            discriminators.dedup();
            assert_eq!(
                discriminators.len(),
                table.len(),
                "{label}: duplicate discriminator"
            );
        }
        let token_2022: Vec<&str> = TOKEN
            .iter()
            .chain(TOKEN_2022_ONLY)
            .map(|spec| spec.name)
            .collect();
        let mut deduped = token_2022.clone();
        deduped.sort_unstable();
        deduped.dedup();
        assert_eq!(
            deduped.len(),
            token_2022.len(),
            "token-2022 types overlap SPL Token"
        );
    }
}
