// Handles Solana system account instructions

use solana_program::system_instruction::SystemInstruction;

pub fn system_instruction_label(instruction: &SystemInstruction) -> String {
    match instruction {
        SystemInstruction::CreateAccount { owner, .. } => {
            format!("Create Account (owner: {owner})")
        }
        SystemInstruction::Assign { owner } => format!("Assign (owner: {owner})"),
        SystemInstruction::Transfer { lamports } => format!("Transfer {lamports} lamports"),
        SystemInstruction::CreateAccountWithSeed { owner, .. } => {
            format!("Create Account With Seed (owner: {owner})")
        }
        SystemInstruction::AdvanceNonceAccount => "Advance Nonce Account".to_string(),
        SystemInstruction::WithdrawNonceAccount(lamports) => {
            format!("Withdraw Nonce Account ({lamports} lamports)")
        }
        SystemInstruction::InitializeNonceAccount(_) => "Initialize Nonce Account".to_string(),
        SystemInstruction::AuthorizeNonceAccount(_) => "Authorize Nonce Account".to_string(),
        SystemInstruction::Allocate { space } => format!("Allocate (space: {space})"),
        SystemInstruction::AllocateWithSeed { owner, .. } => {
            format!("Allocate With Seed (owner: {owner})")
        }
        SystemInstruction::AssignWithSeed { base, seed, owner } => {
            format!("Assign With Seed (base: {base}, seed: {seed}, owner: {owner})")
        }
        SystemInstruction::TransferWithSeed { from_owner, .. } => {
            format!("Transfer With Seed (from_owner: {from_owner})")
        }
        SystemInstruction::UpgradeNonceAccount => "Upgrade Nonce Account".to_string(),
    }
}

// Add more helpers as needed for expanded fields, etc.
