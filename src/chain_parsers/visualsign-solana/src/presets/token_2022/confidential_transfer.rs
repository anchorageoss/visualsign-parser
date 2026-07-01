//! Hand-written decoder for Token-2022 ConfidentialTransfer Transfer/Withdraw
//! (split-proof) sub-instructions.
//!
//! The proof/context accounts are optional and positionally ambiguous: the
//! instruction data carries `*_instruction_offset` fields. An offset of `0`
//! means the proof lives in a context-state account present in the account
//! list; a non-zero offset means the proof is inline in the same transaction,
//! and the instructions sysvar account is present instead. We read the offsets
//! to compute which optional accounts exist, then map positions to named refs.
//!
//! The parser never decrypts: AE balance blobs and auditor ciphertexts are
//! surfaced opaque (base64 / presence bool).
//!
//! `try_decode_confidential_transfer` is consumed by
//! `parse_token_2022_instruction` in `mod.rs`.

use base64::Engine;
use bytemuck::bytes_of;
use spl_token_2022_interface::extension::confidential_transfer::instruction::{
    ConfidentialTransferInstruction, TransferInstructionData, WithdrawInstructionData,
};

/// Outer `TokenInstruction::ConfidentialTransferExtension` discriminator.
pub const CONFIDENTIAL_TRANSFER_EXTENSION_DISCRIMINATOR: u8 = 27;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfidentialTransferIx {
    Withdraw {
        source_token_account: String,
        mint: String,
        owner: String,
        amount: u64,
        decimals: u8,
        new_decryptable_available_balance: String,
        equality_proof_context_account: Option<String>,
        range_proof_context_account: Option<String>,
    },
    Transfer {
        source_token_account: String,
        mint: String,
        destination_token_account: String,
        owner: String,
        new_source_decryptable_available_balance: String,
        auditor_configured: bool,
        equality_proof_context_account: Option<String>,
        validity_proof_context_account: Option<String>,
        range_proof_context_account: Option<String>,
    },
}

fn account_at(accounts: &[String], idx: usize, what: &str) -> Result<String, String> {
    accounts
        .get(idx)
        .cloned()
        .ok_or_else(|| format!("confidential_transfer: missing {what} account at index {idx}"))
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Decode a CT Transfer/Withdraw. `data` is the full instruction data
/// (including the outer `27` discriminator). Returns `Ok(None)` for any other
/// CT sub-instruction so the caller can fall through to existing handling.
pub fn try_decode_confidential_transfer(
    data: &[u8],
    accounts: &[String],
) -> Result<Option<ConfidentialTransferIx>, String> {
    if data.first().copied() != Some(CONFIDENTIAL_TRANSFER_EXTENSION_DISCRIMINATOR) {
        return Ok(None);
    }
    let sub = *data
        .get(1)
        .ok_or_else(|| "confidential_transfer: missing sub-discriminator".to_string())?;
    let body = &data[2..];

    if sub == ConfidentialTransferInstruction::Withdraw as u8 {
        let d: &WithdrawInstructionData = pod_body(body)?;
        let eq_off = d.equality_proof_instruction_offset;
        let range_off = d.range_proof_instruction_offset;

        // Layout: [0]=src, [1]=mint, then optionals [sysvar?, equality?, range?], then owner.
        let mut idx = 2usize;
        if eq_off != 0 || range_off != 0 {
            idx += 1; // skip instructions sysvar
        }
        let equality_proof_context_account = if eq_off == 0 {
            let a = account_at(accounts, idx, "withdraw equality ctx")?;
            idx += 1;
            Some(a)
        } else {
            None
        };
        let range_proof_context_account = if range_off == 0 {
            let a = account_at(accounts, idx, "withdraw range ctx")?;
            idx += 1;
            Some(a)
        } else {
            None
        };

        return Ok(Some(ConfidentialTransferIx::Withdraw {
            source_token_account: account_at(accounts, 0, "withdraw source")?,
            mint: account_at(accounts, 1, "withdraw mint")?,
            owner: account_at(accounts, idx, "withdraw owner")?,
            amount: u64::from(d.amount),
            decimals: d.decimals,
            new_decryptable_available_balance: b64(bytes_of(&d.new_decryptable_available_balance)),
            equality_proof_context_account,
            range_proof_context_account,
        }));
    }

    if sub == ConfidentialTransferInstruction::Transfer as u8 {
        let d: &TransferInstructionData = pod_body(body)?;
        let eq_off = d.equality_proof_instruction_offset;
        let val_off = d.ciphertext_validity_proof_instruction_offset;
        let range_off = d.range_proof_instruction_offset;

        // Layout: [0]=src, [1]=mint, [2]=dest, then [sysvar?, equality?, validity?, range?], then owner.
        let mut idx = 3usize;
        if eq_off != 0 || val_off != 0 || range_off != 0 {
            idx += 1; // skip instructions sysvar
        }
        let equality_proof_context_account = if eq_off == 0 {
            let a = account_at(accounts, idx, "transfer equality ctx")?;
            idx += 1;
            Some(a)
        } else {
            None
        };
        let validity_proof_context_account = if val_off == 0 {
            let a = account_at(accounts, idx, "transfer validity ctx")?;
            idx += 1;
            Some(a)
        } else {
            None
        };
        let range_proof_context_account = if range_off == 0 {
            let a = account_at(accounts, idx, "transfer range ctx")?;
            idx += 1;
            Some(a)
        } else {
            None
        };

        let lo = bytes_of(&d.transfer_amount_auditor_ciphertext_lo);
        let hi = bytes_of(&d.transfer_amount_auditor_ciphertext_hi);
        let auditor_configured = lo.iter().any(|b| *b != 0) || hi.iter().any(|b| *b != 0);

        return Ok(Some(ConfidentialTransferIx::Transfer {
            source_token_account: account_at(accounts, 0, "transfer source")?,
            mint: account_at(accounts, 1, "transfer mint")?,
            destination_token_account: account_at(accounts, 2, "transfer destination")?,
            owner: account_at(accounts, idx, "transfer owner")?,
            new_source_decryptable_available_balance: b64(bytes_of(
                &d.new_source_decryptable_available_balance,
            )),
            auditor_configured,
            equality_proof_context_account,
            validity_proof_context_account,
            range_proof_context_account,
        }));
    }

    // Any other CT sub-instruction is out of scope for this pass.
    Ok(None)
}

/// Cast exactly `size_of::<T>()` bytes from the start of `body` into a Pod `T`.
/// Errors (never panics) on short input or alignment/size mismatch.
fn pod_body<T: bytemuck::Pod>(body: &[u8]) -> Result<&T, String> {
    let size = std::mem::size_of::<T>();
    let slice = body
        .get(..size)
        .ok_or_else(|| format!("confidential_transfer: data too short, need {size} bytes"))?;
    bytemuck::try_from_bytes::<T>(slice)
        .map_err(|e| format!("confidential_transfer: pod cast failed: {e}"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use base64::Engine;
    use bytemuck::bytes_of;
    use spl_token_2022_interface::extension::confidential_transfer::instruction::{
        ConfidentialTransferInstruction, TransferInstructionData, WithdrawInstructionData,
    };

    const WITHDRAW: u8 = ConfidentialTransferInstruction::Withdraw as u8; // 6
    const TRANSFER: u8 = ConfidentialTransferInstruction::Transfer as u8; // 7

    fn accts(labels: &[&str]) -> Vec<String> {
        labels.iter().map(|s| (*s).to_string()).collect()
    }

    // Build raw instruction bytes: [27, sub, <pod struct bytes>].
    fn withdraw_bytes(d: WithdrawInstructionData) -> Vec<u8> {
        let mut v = vec![CONFIDENTIAL_TRANSFER_EXTENSION_DISCRIMINATOR, WITHDRAW];
        v.extend_from_slice(bytes_of(&d));
        v
    }
    fn transfer_bytes(d: TransferInstructionData) -> Vec<u8> {
        let mut v = vec![CONFIDENTIAL_TRANSFER_EXTENSION_DISCRIMINATOR, TRANSFER];
        v.extend_from_slice(bytes_of(&d));
        v
    }

    #[test]
    fn withdraw_context_state_model_maps_accounts() {
        // offsets == 0 => context-state accounts present, no instructions sysvar.
        let d = WithdrawInstructionData {
            amount: 1_000_000u64.into(),
            decimals: 6,
            new_decryptable_available_balance: Default::default(),
            equality_proof_instruction_offset: 0,
            range_proof_instruction_offset: 0,
        };
        // Accounts: [src, mint, equality_ctx, range_ctx, owner]
        let a = accts(&["src", "mint", "eqctx", "rangectx", "owner"]);
        let ix = try_decode_confidential_transfer(&withdraw_bytes(d), &a)
            .unwrap()
            .unwrap();
        match ix {
            ConfidentialTransferIx::Withdraw {
                source_token_account,
                mint,
                owner,
                amount,
                decimals,
                equality_proof_context_account,
                range_proof_context_account,
                new_decryptable_available_balance,
            } => {
                assert_eq!(source_token_account, "src");
                assert_eq!(mint, "mint");
                assert_eq!(owner, "owner");
                assert_eq!(amount, 1_000_000);
                assert_eq!(decimals, 6);
                assert_eq!(equality_proof_context_account.as_deref(), Some("eqctx"));
                assert_eq!(range_proof_context_account.as_deref(), Some("rangectx"));
                // 36-byte zeroed AE ciphertext, base64.
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(&new_decryptable_available_balance)
                    .unwrap();
                assert_eq!(decoded.len(), 36);
            }
            _ => panic!("expected Withdraw"),
        }
    }

    #[test]
    fn withdraw_inline_proof_model_has_sysvar_no_ctx() {
        // offsets != 0 => instructions sysvar present, no context-state accounts.
        let d = WithdrawInstructionData {
            amount: 5u64.into(),
            decimals: 0,
            new_decryptable_available_balance: Default::default(),
            equality_proof_instruction_offset: -2,
            range_proof_instruction_offset: -1,
        };
        // Accounts: [src, mint, sysvar, owner]
        let a = accts(&["src", "mint", "sysvar", "owner"]);
        let ix = try_decode_confidential_transfer(&withdraw_bytes(d), &a)
            .unwrap()
            .unwrap();
        match ix {
            ConfidentialTransferIx::Withdraw {
                owner,
                equality_proof_context_account,
                range_proof_context_account,
                ..
            } => {
                assert_eq!(owner, "owner");
                assert!(equality_proof_context_account.is_none());
                assert!(range_proof_context_account.is_none());
            }
            _ => panic!("expected Withdraw"),
        }
    }

    #[test]
    fn transfer_context_state_model_maps_accounts() {
        let d = TransferInstructionData {
            new_source_decryptable_available_balance: Default::default(),
            transfer_amount_auditor_ciphertext_lo: Default::default(),
            transfer_amount_auditor_ciphertext_hi: Default::default(),
            equality_proof_instruction_offset: 0,
            ciphertext_validity_proof_instruction_offset: 0,
            range_proof_instruction_offset: 0,
        };
        // Accounts: [src, mint, dest, eqctx, validityctx, rangectx, owner]
        let a = accts(&["src", "mint", "dest", "eqctx", "valctx", "rngctx", "owner"]);
        let ix = try_decode_confidential_transfer(&transfer_bytes(d), &a)
            .unwrap()
            .unwrap();
        match ix {
            ConfidentialTransferIx::Transfer {
                source_token_account,
                mint,
                destination_token_account,
                owner,
                auditor_configured,
                equality_proof_context_account,
                validity_proof_context_account,
                range_proof_context_account,
                ..
            } => {
                assert_eq!(source_token_account, "src");
                assert_eq!(mint, "mint");
                assert_eq!(destination_token_account, "dest");
                assert_eq!(owner, "owner");
                assert!(!auditor_configured); // zeroed ciphertexts
                assert_eq!(equality_proof_context_account.as_deref(), Some("eqctx"));
                assert_eq!(validity_proof_context_account.as_deref(), Some("valctx"));
                assert_eq!(range_proof_context_account.as_deref(), Some("rngctx"));
            }
            _ => panic!("expected Transfer"),
        }
    }

    #[test]
    fn non_ct_subinstruction_returns_none() {
        // sub-discriminator 5 (Deposit) is out of scope -> Ok(None).
        let data = vec![CONFIDENTIAL_TRANSFER_EXTENSION_DISCRIMINATOR, 5u8, 0, 0];
        assert!(
            try_decode_confidential_transfer(&data, &accts(&["a", "b"]))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn truncated_data_errors() {
        // sub == Withdraw but no struct bytes.
        let data = vec![CONFIDENTIAL_TRANSFER_EXTENSION_DISCRIMINATOR, WITHDRAW];
        assert!(try_decode_confidential_transfer(&data, &accts(&["a"])).is_err());
    }

    #[test]
    fn missing_accounts_errors() {
        let d = WithdrawInstructionData {
            amount: 1u64.into(),
            decimals: 0,
            new_decryptable_available_balance: Default::default(),
            equality_proof_instruction_offset: 0,
            range_proof_instruction_offset: 0,
        };
        // Only 2 accounts; needs src, mint, eqctx, rangectx, owner.
        let a = accts(&["src", "mint"]);
        assert!(try_decode_confidential_transfer(&withdraw_bytes(d), &a).is_err());
    }
}
