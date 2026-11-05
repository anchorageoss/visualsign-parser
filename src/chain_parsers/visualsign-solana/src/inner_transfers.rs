//! CPI-carried value movement, projected into the same shapes the top-level
//! decode produces. Only the RPC's own `jsonParsed` legs can debit a custodied
//! account, so nothing else is read.

use crate::intermediate::{SolTransfer, SolanaSimulatedInstruction, SplTransfer};

/// Rejects an obviously-wrong `parsed` payload; not key validation.
const MIN_PUBKEY_LEN: usize = 32;

/// Appends CPI transfers in execution order, after the top-level entries.
/// Amounts stay raw base units, as the top-level path emits them.
pub(crate) fn append_simulated_transfers(
    simulated: &[SolanaSimulatedInstruction],
    transfers: &mut Vec<SolTransfer>,
    spl_transfers: &mut Vec<SplTransfer>,
) {
    for instruction in simulated {
        let Some(rpc) = instruction.solana_rpc_parsed_data.as_ref() else {
            continue;
        };
        let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&rpc.parsed_json) else {
            continue;
        };
        let (Some(kind), Some(info)) = (
            parsed.get("type").and_then(serde_json::Value::as_str),
            parsed.get("info"),
        ) else {
            continue;
        };

        match rpc.program.as_str() {
            "system" => {
                if let Some(transfer) = system_transfer(kind, info) {
                    transfers.push(transfer);
                }
            }
            "spl-token" | "spl-token-2022" => {
                if let Some(transfer) = spl_transfer(kind, info) {
                    spl_transfers.push(transfer);
                }
            }
            _ => {}
        }
    }
}

/// System legs that debit lamports. `createAccount` counts: it charges the
/// payer rent. `closeAccount` is absent because it credits.
fn system_transfer(kind: &str, info: &serde_json::Value) -> Option<SolTransfer> {
    let (from_key, to_key) = match kind {
        "transfer" | "transferWithSeed" => ("source", "destination"),
        "createAccount" | "createAccountWithSeed" => ("source", "newAccount"),
        _ => return None,
    };
    Some(SolTransfer {
        from: string_field(info, from_key)?,
        to: string_field(info, to_key)?,
        amount: amount_field(info, "lamports")?,
    })
}

/// Token legs that debit an account. `burn` gets an empty `to`: the tokens
/// leave and nobody receives them. `mintTo` is absent because it credits.
fn spl_transfer(kind: &str, info: &serde_json::Value) -> Option<SplTransfer> {
    let (from, to, amount) = match kind {
        "transfer" | "transferChecked" => (
            string_field(info, "source")?,
            string_field(info, "destination")?,
            token_amount(info)?,
        ),
        "burn" | "burnChecked" => (
            string_field(info, "account")?,
            String::new(),
            token_amount(info)?,
        ),
        _ => return None,
    };

    Some(SplTransfer {
        from,
        to,
        amount,
        // Empty unless the RPC names an owner: `authority` may be a delegate,
        // and a wrong owner would key a balance lookup to the wrong wallet.
        owner: string_field(info, "owner").unwrap_or_default(),
        signers: info
            .get("signers")
            .and_then(serde_json::Value::as_array)
            .map(|signers| {
                signers
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        token_mint: string_field(info, "mint"),
        decimals: decimals_field(info),
        // Only `transferCheckedWithFee` carries one; absent elsewhere.
        fee: info
            .get("feeAmount")
            .and_then(|fee| fee.get("amount"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
    })
}

/// SPL amounts arrive either as a bare `amount` string (`transfer`, `burn`) or
/// nested under `tokenAmount` (`transferChecked`, `burnChecked`).
fn token_amount(info: &serde_json::Value) -> Option<String> {
    if let Some(amount) = amount_field(info, "amount") {
        return Some(amount);
    }
    info.get("tokenAmount")
        .and_then(|token| token.get("amount"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

fn decimals_field(info: &serde_json::Value) -> Option<String> {
    if let Some(decimals) = info.get("decimals").and_then(serde_json::Value::as_u64) {
        return Some(decimals.to_string());
    }
    info.get("tokenAmount")
        .and_then(|token| token.get("decimals"))
        .and_then(serde_json::Value::as_u64)
        .map(|decimals| decimals.to_string())
}

/// A pubkey-shaped field; short values would name a meaningless address.
fn string_field(info: &serde_json::Value, key: &str) -> Option<String> {
    info.get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|value| value.len() >= MIN_PUBKEY_LEN)
        .map(str::to_string)
}

/// Lamports arrive as JSON numbers, token amounts as strings (they exceed
/// 2^53). Both normalize to the decimal string the top-level path emits.
fn amount_field(info: &serde_json::Value, key: &str) -> Option<String> {
    match info.get(key)? {
        serde_json::Value::String(value) => Some(value.clone()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::intermediate::{RegisteredSource, SolanaRpcParsedInstructionDataIo};

    fn rpc_instruction(
        program: &str,
        stack_height: u32,
        parsed_json: &str,
    ) -> SolanaSimulatedInstruction {
        SolanaSimulatedInstruction {
            index: 2,
            stack_height,
            program_key: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_string(),
            accounts: Vec::new(),
            instruction_data_hex: String::new(),
            registered_source: RegisteredSource::Native,
            parsed_instruction_data: None,
            solana_rpc_parsed_data: Some(SolanaRpcParsedInstructionDataIo {
                program: program.to_string(),
                parsed_json: parsed_json.to_string(),
            }),
            idl_parse_error: None,
        }
    }

    fn run(simulated: &[SolanaSimulatedInstruction]) -> (Vec<SolTransfer>, Vec<SplTransfer>) {
        let mut transfers = Vec::new();
        let mut spl_transfers = Vec::new();
        append_simulated_transfers(simulated, &mut transfers, &mut spl_transfers);
        (transfers, spl_transfers)
    }

    /// Real inner leg from the PROF-354 Jupiter swap: the 13.13 USDC debit
    /// the top-level walk never sees.
    #[test]
    fn jupiter_swap_inner_spl_transfer_is_lifted() {
        let (transfers, spl) = run(&[rpc_instruction(
            "spl-token",
            3,
            r#"{"type":"transfer","info":{
                "amount":"13133749",
                "authority":"Eb1Z35D11dmR6SFYoq87YnWN2aHpsAmeLYMwMm4oz9ZB",
                "destination":"DvFNh2UGbzaj7BBGPHv98g646EA9k5Tj6YHPnLHEZFPm",
                "source":"GfXcftCa3AEms45sksSokT8myQKTd252JUqeq9ji8pwk"}}"#,
        )]);

        assert!(transfers.is_empty());
        assert_eq!(spl.len(), 1);
        assert_eq!(spl[0].amount, "13133749");
        assert_eq!(spl[0].from, "GfXcftCa3AEms45sksSokT8myQKTd252JUqeq9ji8pwk");
        assert_eq!(spl[0].to, "DvFNh2UGbzaj7BBGPHv98g646EA9k5Tj6YHPnLHEZFPm");
        // A plain `transfer` names no mint and no owner.
        assert_eq!(spl[0].token_mint, None);
        assert_eq!(spl[0].owner, "");
    }

    /// Jupiter Lend withdraw: the only debit is a burn, going nowhere.
    #[test]
    fn burn_is_reported_as_a_debit_with_no_destination() {
        let (_, spl) = run(&[rpc_instruction(
            "spl-token",
            2,
            r#"{"type":"burn","info":{
                "account":"9h9upduap2YH1Y3baU5RuDdL3xHbmKdizvCvXDCYHzLS",
                "amount":"41383636",
                "authority":"3HHr4K4rAYK1vJ7vfRCmbr4DeZa1ypaki29hZGezYp69",
                "mint":"9BEcn9aPEmhSPbPQeFGjidRiEKki46fVQDyPpSQXPA2D"}}"#,
        )]);

        assert_eq!(spl.len(), 1);
        assert_eq!(spl[0].from, "9h9upduap2YH1Y3baU5RuDdL3xHbmKdizvCvXDCYHzLS");
        assert_eq!(spl[0].to, "");
        assert_eq!(spl[0].amount, "41383636");
        assert_eq!(
            spl[0].token_mint.as_deref(),
            Some("9BEcn9aPEmhSPbPQeFGjidRiEKki46fVQDyPpSQXPA2D")
        );
    }

    /// `transferChecked` nests its amount and carries the mint, unlike `transfer`.
    #[test]
    fn transfer_checked_carries_mint_and_decimals() {
        let (_, spl) = run(&[rpc_instruction(
            "spl-token",
            2,
            r#"{"type":"transferChecked","info":{
                "authority":"DASPvsk4Pt7o3kRKdJGc33Bv43F6Wdx96zsPaqoD3nV1",
                "destination":"BmkUoKMFYBxNSzWXyUjyMJjMAaVz4d8ZnxwwmhDCUXFB",
                "mint":"EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
                "source":"GE8BTcoV5T3SfWmQUB9A3hKhi5RiFmMbmffyux3xWoRL",
                "tokenAmount":{"amount":"414122446","decimals":6,"uiAmountString":"414.122446"}}}"#,
        )]);

        assert_eq!(spl.len(), 1);
        assert_eq!(spl[0].amount, "414122446");
        assert_eq!(spl[0].decimals.as_deref(), Some("6"));
        assert_eq!(
            spl[0].token_mint.as_deref(),
            Some("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v")
        );
    }

    /// An ATA create debits the payer for rent, transfer or not.
    #[test]
    fn system_create_account_is_a_lamport_debit() {
        let (transfers, _) = run(&[rpc_instruction(
            "system",
            3,
            r#"{"type":"createAccount","info":{
                "source":"DASPvsk4Pt7o3kRKdJGc33Bv43F6Wdx96zsPaqoD3nV1",
                "newAccount":"678f85kKQLNkg6eNhnUmTRXk3Z4LCSKgsVAGW5KPtvq",
                "lamports":2039280,
                "space":165}}"#,
        )]);

        assert_eq!(transfers.len(), 1);
        assert_eq!(transfers[0].amount, "2039280");
        assert_eq!(
            transfers[0].to,
            "678f85kKQLNkg6eNhnUmTRXk3Z4LCSKgsVAGW5KPtvq"
        );
    }

    /// Credits cannot fund anything, so they never become entries.
    #[test]
    fn credits_are_not_transfers() {
        let (transfers, spl) = run(&[
            rpc_instruction(
                "spl-token",
                2,
                r#"{"type":"mintTo","info":{
                    "account":"CqyxNiPtxpcFfa5ftCAkRxHV8MjFhy5tohfJaJugfviN",
                    "amount":"503192176",
                    "mint":"GcV9tEj62VncGithz4o4N9x6HWXARxuRgEAYk9zahNA8",
                    "mintAuthority":"5nmGjA4s7ATzpBQXC5RNceRpaJ7pYw2wKsNBWyuSAZV6"}}"#,
            ),
            rpc_instruction(
                "spl-token",
                2,
                r#"{"type":"closeAccount","info":{
                    "account":"GE8BTcoV5T3SfWmQUB9A3hKhi5RiFmMbmffyux3xWoRL",
                    "destination":"DASPvsk4Pt7o3kRKdJGc33Bv43F6Wdx96zsPaqoD3nV1",
                    "owner":"DASPvsk4Pt7o3kRKdJGc33Bv43F6Wdx96zsPaqoD3nV1"}}"#,
            ),
        ]);

        assert!(transfers.is_empty());
        assert!(spl.is_empty());
    }

    /// Opaque and IDL-decoded legs move nothing the caller owns.
    #[test]
    fn non_rpc_parsed_instructions_are_skipped() {
        let mut opaque = rpc_instruction("spl-token", 2, "{}");
        opaque.solana_rpc_parsed_data = None;
        opaque.registered_source = RegisteredSource::Unregistered;

        let (transfers, spl) = run(&[opaque]);
        assert!(transfers.is_empty());
        assert!(spl.is_empty());
    }

    /// A `parsed` payload missing fields must not half-build a transfer.
    #[test]
    fn malformed_parsed_payloads_are_dropped() {
        let (transfers, spl) = run(&[
            rpc_instruction("spl-token", 2, r#"{"type":"transfer","info":{}}"#),
            rpc_instruction(
                "spl-token",
                2,
                r#"{"type":"transfer","info":{"source":"x","destination":"y","amount":"1"}}"#,
            ),
            rpc_instruction("system", 2, r#"{"type":"transfer"}"#),
            rpc_instruction("spl-token", 2, "not json"),
        ]);

        assert!(transfers.is_empty());
        assert!(spl.is_empty());
    }

    /// Order is the contract: consumers read these as a debit sequence.
    #[test]
    fn execution_order_is_preserved() {
        let (_, spl) = run(&[
            rpc_instruction(
                "spl-token",
                2,
                r#"{"type":"burn","info":{"account":"9h9upduap2YH1Y3baU5RuDdL3xHbmKdizvCvXDCYHzLS","amount":"1"}}"#,
            ),
            rpc_instruction(
                "spl-token",
                3,
                r#"{"type":"transfer","info":{"source":"GfXcftCa3AEms45sksSokT8myQKTd252JUqeq9ji8pwk","destination":"DvFNh2UGbzaj7BBGPHv98g646EA9k5Tj6YHPnLHEZFPm","amount":"2"}}"#,
            ),
        ]);

        assert_eq!(spl.len(), 2);
        assert_eq!(spl[0].amount, "1");
        assert_eq!(spl[1].amount, "2");
    }
}
