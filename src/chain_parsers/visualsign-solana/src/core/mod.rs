use std::collections::BTreeMap;

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

/// Maximum CPI nesting depth. Solana's runtime hard-caps CPI at 4 levels, so
/// any instruction chain deeper than this cannot execute on-chain. Visualizers
/// that recurse into inner instructions (e.g. swig_wallet, squads) should bail
/// at this bound rather than rendering content that can't exist in production.
pub const MAX_CALL_DEPTH: usize = 4;

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
///
/// # Resolution patterns
///
/// Two ways for a visualizer to handle indices that don't resolve against
/// `account_keys` (out-of-bounds, or v0 lookup-table entries that haven't
/// been resolved):
///
/// **All-or-nothing (most IDL-based presets).**
/// Use `resolve_program_id()` / `resolve_accounts()`. The first unresolved
/// index aborts visualization with a precise `Err` naming the bad index.
/// Suitable when downstream parsing requires every account to be a real
/// pubkey -- e.g. an Anchor IDL parser building a `named_accounts` map.
///
/// **Partial rendering (catch-all visualizers).**
/// Pattern-match on `program_id()` and `account(n)` directly and substitute
/// a placeholder for unresolved indices instead of erroring. The
/// `unknown_program` preset is the canonical example: it renders
/// `unresolved(N)` strings so the user still sees *something* for an
/// instruction no specific visualizer could handle.
#[derive(Debug, Clone)]
pub struct VisualizerContext<'a> {
    sender: &'a SolanaAccount,
    compiled_instruction: &'a solana_sdk::instruction::CompiledInstruction,
    account_keys: &'a [Pubkey],
    idl_registry: &'a crate::idl::IdlRegistry,
    instruction_index: usize,
    /// CPI nesting depth for visualizers that re-enter `visualize_with_any`.
    ///
    /// Top-level instructions start at depth 0. Visualizers that decode and
    /// recursively visualize inner instructions (e.g. swig_wallet's
    /// `SignV1`/`SignV2` payloads) must increment this when building a child
    /// context, so the recursion can be bounded at the trait boundary.
    call_depth: usize,
}

impl<'a> VisualizerContext<'a> {
    pub fn new(
        sender: &'a SolanaAccount,
        compiled_instruction: &'a solana_sdk::instruction::CompiledInstruction,
        account_keys: &'a [Pubkey],
        idl_registry: &'a crate::idl::IdlRegistry,
        instruction_index: usize,
    ) -> Self {
        Self {
            sender,
            compiled_instruction,
            account_keys,
            idl_registry,
            instruction_index,
            call_depth: 0,
        }
    }

    /// Set the CPI call depth for this context. Returns the modified context
    /// so it can be chained at construction sites: `VisualizerContext::new(...)
    /// .with_call_depth(parent.call_depth().saturating_add(1))`.
    ///
    /// Use `saturating_add` (not `+`) when deriving from a parent depth: an
    /// arithmetic `+` overflows `usize` on a pathologically large parent depth
    /// and wraps to 0 in release builds, which would reintroduce unbounded
    /// recursion.
    ///
    /// `#[must_use]`: dropping the returned context silently leaves call_depth
    /// at 0, which would reintroduce unbounded recursion at the next trait
    /// boundary.
    #[must_use]
    pub fn with_call_depth(mut self, call_depth: usize) -> Self {
        self.call_depth = call_depth;
        self
    }

    pub fn idl_registry(&self) -> &crate::idl::IdlRegistry {
        self.idl_registry
    }

    pub fn sender(&self) -> &SolanaAccount {
        self.sender
    }

    /// Position of this instruction in the transaction's instruction list.
    pub fn instruction_index(&self) -> usize {
        self.instruction_index
    }

    /// CPI call depth of this context. 0 for top-level instructions; child
    /// contexts produced by inner-instruction visualizers should set
    /// `parent.call_depth().saturating_add(1)` via `with_call_depth` (not `+`,
    /// which can wrap `usize` to 0 in release builds and bypass the bound).
    pub fn call_depth(&self) -> usize {
        self.call_depth
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

    /// Resolve the program_id, returning Err if the index is out of bounds.
    /// For visualizers that can't proceed without a known program.
    pub fn resolve_program_id(&self) -> Result<Pubkey, VisualSignError> {
        match self.program_id() {
            ProgramRef::Resolved(pk) => Ok(*pk),
            ProgramRef::Unresolved { raw_index } => Err(VisualSignError::DecodeError(format!(
                "unresolved program at index {raw_index}"
            ))),
        }
    }

    /// Resolve every account index in the instruction to an AccountMeta,
    /// returning Err on the first unresolved index. Writable/signer bits are
    /// not currently surfaced (set to false); IDL-based presets only need pubkeys.
    pub fn resolve_accounts(
        &self,
    ) -> Result<Vec<solana_sdk::instruction::AccountMeta>, VisualSignError> {
        (0..self.num_accounts())
            .map(|i| match self.account(i) {
                Some(AccountRef::Resolved(pk)) => Ok(
                    solana_sdk::instruction::AccountMeta::new_readonly(*pk, false),
                ),
                Some(AccountRef::Unresolved { raw_index }) => Err(VisualSignError::DecodeError(
                    format!("unresolved account index {raw_index} at position {i}"),
                )),
                None => Err(VisualSignError::DecodeError(format!(
                    "missing account at position {i}"
                ))),
            })
            .collect()
    }
}

pub struct SolanaIntegrationConfigData {
    pub programs: BTreeMap<&'static str, BTreeMap<&'static str, Vec<&'static str>>>,
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
        let ci = CompiledInstruction {
            program_id_index: 1,
            accounts: vec![0],
            data: vec![0xAA],
        };
        let sender = SolanaAccount {
            account_key: keys[0].to_string(),
            signer: false,
            writable: false,
        };
        let registry = crate::idl::IdlRegistry::new();
        let ctx = VisualizerContext::new(&sender, &ci, &keys, &registry, 0);
        assert_eq!(ctx.program_id(), ProgramRef::Resolved(&keys[1]));
    }

    #[test]
    fn test_program_id_unresolved() {
        let keys = vec![Pubkey::new_unique()];
        let ci = CompiledInstruction {
            program_id_index: 99,
            accounts: vec![],
            data: vec![],
        };
        let sender = SolanaAccount {
            account_key: keys[0].to_string(),
            signer: false,
            writable: false,
        };
        let registry = crate::idl::IdlRegistry::new();
        let ctx = VisualizerContext::new(&sender, &ci, &keys, &registry, 0);
        assert_eq!(ctx.program_id(), ProgramRef::Unresolved { raw_index: 99 });
    }

    #[test]
    fn test_account_resolved_and_unresolved() {
        let keys = vec![Pubkey::new_unique(), Pubkey::new_unique()];
        let ci = CompiledInstruction {
            program_id_index: 1,
            accounts: vec![0, 50],
            data: vec![],
        };
        let sender = SolanaAccount {
            account_key: keys[0].to_string(),
            signer: false,
            writable: false,
        };
        let registry = crate::idl::IdlRegistry::new();
        let ctx = VisualizerContext::new(&sender, &ci, &keys, &registry, 0);
        assert_eq!(ctx.account(0), Some(AccountRef::Resolved(&keys[0])));
        assert_eq!(
            ctx.account(1),
            Some(AccountRef::Unresolved { raw_index: 50 })
        );
        assert_eq!(ctx.account(99), None); // no such position
    }

    #[test]
    fn test_data_and_num_accounts() {
        let keys = vec![Pubkey::new_unique()];
        let ci = CompiledInstruction {
            program_id_index: 0,
            accounts: vec![0, 0, 0],
            data: vec![0xDE, 0xAD],
        };
        let sender = SolanaAccount {
            account_key: keys[0].to_string(),
            signer: false,
            writable: false,
        };
        let registry = crate::idl::IdlRegistry::new();
        let ctx = VisualizerContext::new(&sender, &ci, &keys, &registry, 0);
        assert_eq!(ctx.data(), &[0xDE, 0xAD]);
        assert_eq!(ctx.num_accounts(), 3);
    }

    #[test]
    fn test_call_depth_default_and_setter() {
        let keys = vec![Pubkey::new_unique()];
        let ci = CompiledInstruction {
            program_id_index: 0,
            accounts: vec![],
            data: vec![],
        };
        let sender = SolanaAccount {
            account_key: keys[0].to_string(),
            signer: false,
            writable: false,
        };
        let registry = crate::idl::IdlRegistry::new();
        let ctx = VisualizerContext::new(&sender, &ci, &keys, &registry, 0);
        assert_eq!(ctx.call_depth(), 0);
        let ctx = ctx.with_call_depth(MAX_CALL_DEPTH);
        assert_eq!(ctx.call_depth(), MAX_CALL_DEPTH);
        let ctx = ctx.with_call_depth(usize::MAX);
        assert_eq!(ctx.call_depth(), usize::MAX);
    }
}
