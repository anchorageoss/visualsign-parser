use std::collections::HashMap;

use ::visualsign::AnnotatedPayloadField;
use ::visualsign::errors::VisualSignError;
use solana_parser::solana::structs::SolanaAccount;
use solana_sdk::pubkey::Pubkey;

mod accounts;
mod instructions;
mod txtypes;
mod visualsign;

pub use accounts::*;
pub use instructions::*;
pub use txtypes::*;
pub use visualsign::*;

/// Identifier for which visualizer handled a command, categorized by dApp type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VisualizerKind {
    /// Decentralized exchange protocols (e.g., AMMs, DEX aggregators)
    Dex(&'static str),
    /// Lending/borrowing protocols
    Lending(&'static str),
    /// Validator or pooled staking without liquid derivative tokens
    StakingPools(&'static str),
    /// Payment and simple transfer-related operations
    Payments(&'static str),
}

/// Resolution of a compiled instruction's program_id_index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgramRef<'a> {
    Resolved(&'a Pubkey),
    Unresolved { raw_index: u8 },
}

/// Resolution of a compiled instruction's account index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountRef<'a> {
    Resolved(&'a Pubkey),
    Unresolved { raw_index: u8 },
}

/// Context for visualizing a Solana instruction.
///
/// Holds references to the transaction's wire data -- no copies.
/// Resolution of compiled instruction indices to pubkeys happens
/// lazily via helper methods.
#[derive(Debug, Clone)]
pub struct VisualizerContext<'a> {
    sender: &'a SolanaAccount,
    compiled_instruction: &'a solana_sdk::instruction::CompiledInstruction,
    account_keys: &'a [Pubkey],
    idl_registry: &'a crate::idl::IdlRegistry,
}

impl<'a> VisualizerContext<'a> {
    pub fn new(
        sender: &'a SolanaAccount,
        compiled_instruction: &'a solana_sdk::instruction::CompiledInstruction,
        account_keys: &'a [Pubkey],
        idl_registry: &'a crate::idl::IdlRegistry,
    ) -> Self {
        Self {
            sender,
            compiled_instruction,
            account_keys,
            idl_registry,
        }
    }

    pub fn idl_registry(&self) -> &crate::idl::IdlRegistry {
        self.idl_registry
    }

    pub fn sender(&self) -> &SolanaAccount {
        self.sender
    }

    /// Resolves the program_id_index. Every compiled instruction has one,
    /// so this always returns a value -- either resolved or unresolved.
    pub fn program_id(&self) -> ProgramRef<'a> {
        let idx = self.compiled_instruction.program_id_index;
        match self.account_keys.get(idx as usize) {
            Some(pk) => ProgramRef::Resolved(pk),
            None => ProgramRef::Unresolved { raw_index: idx },
        }
    }

    /// Resolves the account at `position` in the instruction's accounts list.
    /// Returns None if the instruction has no account at this position.
    /// Returns Resolved or Unresolved for accounts the instruction does reference.
    pub fn account(&self, position: usize) -> Option<AccountRef<'a>> {
        let &idx = self.compiled_instruction.accounts.get(position)?;
        Some(match self.account_keys.get(idx as usize) {
            Some(pk) => AccountRef::Resolved(pk),
            None => AccountRef::Unresolved { raw_index: idx },
        })
    }

    /// Raw instruction data bytes. No copy.
    pub fn data(&self) -> &'a [u8] {
        &self.compiled_instruction.data
    }

    /// Number of account references in this instruction.
    pub fn num_accounts(&self) -> usize {
        self.compiled_instruction.accounts.len()
    }

    /// Reference to the underlying compiled instruction.
    pub fn compiled_instruction(&self) -> &'a solana_sdk::instruction::CompiledInstruction {
        self.compiled_instruction
    }

    /// Reference to the account keys array.
    pub fn account_keys(&self) -> &'a [Pubkey] {
        self.account_keys
    }
}

pub struct SolanaIntegrationConfigData {
    pub programs: HashMap<&'static str, HashMap<&'static str, Vec<&'static str>>>,
}

pub trait SolanaIntegrationConfig {
    fn new() -> Self
    where
        Self: Sized;

    fn data(&self) -> &SolanaIntegrationConfigData;

    fn can_handle(&self, program_id: &str) -> bool {
        self.data()
            .programs
            .get(program_id)
            .map(|_supported_instructions| true)
            .unwrap_or(false)
    }
}

pub trait InstructionVisualizer {
    fn visualize_tx_commands(
        &self,
        context: &VisualizerContext,
    ) -> Result<AnnotatedPayloadField, VisualSignError>;

    fn get_config(&self) -> Option<&dyn SolanaIntegrationConfig>;

    fn kind(&self) -> VisualizerKind;

    fn can_handle(&self, context: &VisualizerContext) -> bool {
        let Some(config) = self.get_config() else {
            return false;
        };

        match context.program_id() {
            ProgramRef::Resolved(pk) => config.can_handle(&pk.to_string()),
            ProgramRef::Unresolved { .. } => false,
        }
    }
}

/// Result of a successful visualization attempt, including which visualizer handled it.
#[derive(Debug, Clone)]
pub struct VisualizeResult {
    pub field: AnnotatedPayloadField,
    pub kind: VisualizerKind,
}

/// Tries multiple visualizers in order, returning the first successful visualization.
pub fn visualize_with_any(
    visualizers: &[&dyn InstructionVisualizer],
    context: &VisualizerContext,
) -> Option<Result<VisualizeResult, VisualSignError>> {
    visualizers.iter().find_map(|v| {
        if !v.can_handle(context) {
            return None;
        }

        Some(
            v.visualize_tx_commands(context)
                .map(|field| VisualizeResult {
                    field,
                    kind: v.kind(),
                }),
        )
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use solana_sdk::instruction::CompiledInstruction;

    #[test]
    fn test_program_id_resolved() {
        let keys = vec![Pubkey::new_unique(), Pubkey::new_unique()];
        let ci = CompiledInstruction { program_id_index: 1, accounts: vec![0], data: vec![0xAA] };
        let sender = SolanaAccount { account_key: keys[0].to_string(), signer: false, writable: false };
        let registry = crate::idl::IdlRegistry::new();
        let ctx = VisualizerContext::new(&sender, &ci, &keys, &registry);
        assert_eq!(ctx.program_id(), ProgramRef::Resolved(&keys[1]));
    }

    #[test]
    fn test_program_id_unresolved() {
        let keys = vec![Pubkey::new_unique()];
        let ci = CompiledInstruction { program_id_index: 99, accounts: vec![], data: vec![] };
        let sender = SolanaAccount { account_key: keys[0].to_string(), signer: false, writable: false };
        let registry = crate::idl::IdlRegistry::new();
        let ctx = VisualizerContext::new(&sender, &ci, &keys, &registry);
        assert_eq!(ctx.program_id(), ProgramRef::Unresolved { raw_index: 99 });
    }

    #[test]
    fn test_account_resolved_and_unresolved() {
        let keys = vec![Pubkey::new_unique(), Pubkey::new_unique()];
        let ci = CompiledInstruction { program_id_index: 1, accounts: vec![0, 50], data: vec![] };
        let sender = SolanaAccount { account_key: keys[0].to_string(), signer: false, writable: false };
        let registry = crate::idl::IdlRegistry::new();
        let ctx = VisualizerContext::new(&sender, &ci, &keys, &registry);
        assert_eq!(ctx.account(0), Some(AccountRef::Resolved(&keys[0])));
        assert_eq!(ctx.account(1), Some(AccountRef::Unresolved { raw_index: 50 }));
        assert_eq!(ctx.account(99), None); // no such position
    }

    #[test]
    fn test_data_and_num_accounts() {
        let keys = vec![Pubkey::new_unique()];
        let ci = CompiledInstruction { program_id_index: 0, accounts: vec![0, 0, 0], data: vec![0xDE, 0xAD] };
        let sender = SolanaAccount { account_key: keys[0].to_string(), signer: false, writable: false };
        let registry = crate::idl::IdlRegistry::new();
        let ctx = VisualizerContext::new(&sender, &ci, &keys, &registry);
        assert_eq!(ctx.data(), &[0xDE, 0xAD]);
        assert_eq!(ctx.num_accounts(), 3);
    }
}
