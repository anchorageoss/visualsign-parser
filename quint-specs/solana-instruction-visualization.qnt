// -*- mode: Bluespec; -*-
/// Solana instruction visualization: how each instruction of a decoded Solana
/// transaction is routed to a visualizer and rendered into a signable payload
/// field, and what happens to the payload when a visualizer fails.
///
/// Modeled from:
///   * core/mod.rs -- `InstructionVisualizer::can_handle` (consults ONLY the
///     program id; the per-instruction map in `SolanaIntegrationConfigData` is
///     never read) and `visualize_with_any` (first visualizer whose
///     `can_handle` is true wins).
///   * build.rs -- `collect_visualizers` forces `unknown_program` last; its
///     `can_handle` returns true unconditionally, so it is the catch-all.
///   * core/txtypes/v0.rs -- `decode_v0_instructions`. The production build
///     (parser_app, `diagnostics` OFF) returns `Err` for the WHOLE transaction
///     on the first per-instruction visualizer error.
///   * presets/spl_token/mod.rs -- documents the "partial rendering" contract:
///     a preset degrades malformed bytes to a raw layout instead of returning
///     `Err`, precisely because the non-diagnostics dispatch turns an `Err`
///     into a whole-transaction failure.
///   * presets/metadao_futarchy/mod.rs -- the IDL-preset pattern, which does
///     NOT follow that contract: it returns `Err` on data shorter than the
///     8-byte Anchor discriminator and on a discriminator absent from the IDL.
module solana_instruction_visualization {

  // ----------------------------------------------------------------- domain

  /// Program ids that can appear as an instruction's program. Real base58 ids
  /// for the programs that have a preset; a placeholder for "some program with
  /// no preset registered", and one for a program_id_index that does not
  /// resolve against `account_keys` (v0 + address lookup table).
  pure val OKX_ROUTER_PROGRAM: str = "proVF4pMXVaYqmy4NjniPh4pqKNfMmsihgd4wdkCX3u"
  pure val SPL_TOKEN_PROGRAM: str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
  pure val METADAO_PROGRAM: str = "FUTARELBfJfQ8RDGhg1wdhddq1odMAJUePHFuBYfUxKq"
  pure val UNREGISTERED_PROGRAM: str = "UnregisteredProgram11111111111111111111111"
  pure val UNRESOLVED_PROGRAM: str = "unresolved"

  pure val PROGRAM_IDS: Set[str] = Set(
    OKX_ROUTER_PROGRAM,
    SPL_TOKEN_PROGRAM,
    METADAO_PROGRAM,
    UNREGISTERED_PROGRAM,
    UNRESOLVED_PROGRAM,
  )

  /// Visualizer names as they appear in `available_visualizers()`.
  /// `unknown_program` is the catch-all and is always last.
  pure val UNKNOWN_VISUALIZER: str = "unknown_program"
  /// A "partial rendering" preset: never returns Err (see spl_token docs).
  pure val SPL_TOKEN_VISUALIZER: str = "spl_token"
  /// An IDL-backed preset: returns Err on short data / unknown discriminator.
  pure val METADAO_VISUALIZER: str = "metadao_futarchy"

  /// Clamped abstractions of the real wire data, so every value a real trace
  /// can log lies inside the modeled domain. The code logs
  /// `min(instruction.data.len(), MAX_DATA_LEN)` and
  /// `min(accounts.len(), MAX_ACCOUNTS)`; the only thresholds the dispatch
  /// path tests are 1 and 8, both well inside the range.
  pure val MAX_DATA_LEN: int = 16
  pure val MAX_ACCOUNTS: int = 8

  /// What a visualizer produced for one instruction.
  ///   "decoded"    -- named instruction, named accounts, decoded args
  ///   "raw"        -- catch-all raw program id + hex bytes
  ///   "unparsable" -- preset could not parse, degraded to a raw layout
  ///   "error"      -- visualizer returned Err
  ///   "none"       -- no instruction visualized yet
  type Rendered = {
    status: str,
    namedAccounts: int,
  }

  type Outcome = {
    program: str,
    visualizer: str,
    status: str,
    namedAccounts: int,
  }

  /// One (program, visualizer, status) triple that has been observed. The
  /// accumulator is write-only: nothing in the transition relation reads it,
  /// it exists so behavior observations are sticky across a whole run.
  type Mark = {
    program: str,
    visualizer: str,
    status: str,
  }

  pure val NO_OUTCOME: Outcome = {
    program: "",
    visualizer: "",
    status: "none",
    namedAccounts: 0,
  }

  // --------------------------------------------------------------- dispatch

  pure def minInt(a: int, b: int): int = if (a < b) a else b

  /// `visualize_with_any` returns the first visualizer whose `can_handle` is
  /// true. `can_handle` resolves the program id and looks it up in the
  /// visualizer's `programs` map; an unresolved program id matches nothing.
  /// `unknown_program` is last and answers true for everything.
  pure def selectVisualizer(programId: str): str =
    if (programId == SPL_TOKEN_PROGRAM) SPL_TOKEN_VISUALIZER
    else if (programId == METADAO_PROGRAM) METADAO_VISUALIZER
    else UNKNOWN_VISUALIZER

  /// What the selected visualizer produces for this instruction.
  pure def renderInstruction(
    visualizer: str,
    dataLen: int,
    discriminatorKnown: bool,
    allAccountsResolved: bool,
    accountCount: int,
    idlAccountCount: int,
    registryHasIdl: bool,
  ): Rendered =
    if (visualizer == UNKNOWN_VISUALIZER)
      // Catch-all: tries a caller-supplied IDL from the registry first, and
      // otherwise renders raw bytes with `unresolved(N)` placeholders for
      // indices it cannot resolve. It never returns Err.
      if (registryHasIdl and discriminatorKnown and dataLen >= 8)
        { status: "decoded", namedAccounts: minInt(accountCount, idlAccountCount) }
      else
        { status: "raw", namedAccounts: 0 }
    else if (visualizer == SPL_TOKEN_VISUALIZER)
      // Partial-rendering preset: unparsable bytes degrade to a raw layout
      // rather than aborting the transaction.
      if (discriminatorKnown and dataLen >= 1)
        { status: "decoded", namedAccounts: minInt(accountCount, idlAccountCount) }
      else
        { status: "unparsable", namedAccounts: 0 }
    else if (visualizer == METADAO_VISUALIZER)
      // IDL preset: fails closed on anything the bundled IDL cannot decode.
      if (dataLen < 8) { status: "error", namedAccounts: 0 }
      else if (not(discriminatorKnown)) { status: "error", namedAccounts: 0 }
      else { status: "decoded", namedAccounts: minInt(accountCount, idlAccountCount) }
    else
      { status: "error", namedAccounts: 0 }

  // ------------------------------------------------------------------ state

  /// Sticky accumulator of every (program, visualizer, status) observed.
  var seen: Set[Mark]
  /// A transaction decode is in progress.
  var txOpen: bool
  /// Instructions visualized so far in the current transaction.
  var instructionsProcessed: int
  /// Payload fields produced so far in the current transaction.
  var fieldsRendered: int
  /// Production build: a visualizer error aborted this transaction's payload.
  var payloadAborted: bool
  /// The most recent instruction's outcome.
  var lastOutcome: Outcome

  action specInit: bool = all {
    seen' = Set(),
    txOpen' = false,
    instructionsProcessed' = 0,
    fieldsRendered' = 0,
    payloadAborted' = false,
    lastOutcome' = NO_OUTCOME,
  }

  /// `decode_v0_instructions` entry: a fresh transaction starts decoding.
  /// Repeatable -- one process decodes many transactions in sequence.
  action beginTransaction: bool = all {
    not(txOpen),
    txOpen' = true,
    instructionsProcessed' = 0,
    fieldsRendered' = 0,
    payloadAborted' = false,
    lastOutcome' = NO_OUTCOME,
    seen' = seen,
  }

  /// One iteration of the per-instruction loop in `decode_v0_instructions`:
  /// build the context, pick a visualizer, render.
  ///
  /// Not enabled once `payloadAborted` holds: in the production build the
  /// first `Some(Err(_))` returns immediately, so later instructions are
  /// never visualized.
  action visualizeInstruction(
    programId: str,
    dataLen: int,
    discriminatorKnown: bool,
    allAccountsResolved: bool,
    accountCount: int,
    idlAccountCount: int,
    registryHasIdl: bool,
  ): bool = {
    val visualizer = selectVisualizer(programId)
    val rendered = renderInstruction(
      visualizer, dataLen, discriminatorKnown, allAccountsResolved,
      accountCount, idlAccountCount, registryHasIdl,
    )
    all {
      txOpen,
      not(payloadAborted),
      // A recognised selector needs at least one byte on the wire.
      (discriminatorKnown implies (dataLen >= 1)),
      instructionsProcessed' = instructionsProcessed + 1,
      fieldsRendered' =
        if (rendered.status == "error") fieldsRendered else fieldsRendered + 1,
      payloadAborted' = (rendered.status == "error"),
      lastOutcome' = {
        program: programId,
        visualizer: visualizer,
        status: rendered.status,
        namedAccounts: rendered.namedAccounts,
      },
      seen' = seen.union(Set({
        program: programId,
        visualizer: visualizer,
        status: rendered.status,
      })),
      txOpen' = txOpen,
    }
  }

  /// `decode_v0_instructions` returns -- either the field list or the
  /// whole-transaction `Err` produced by the first failing instruction.
  action finishTransaction: bool = all {
    txOpen,
    txOpen' = false,
    seen' = seen,
    instructionsProcessed' = instructionsProcessed,
    fieldsRendered' = fieldsRendered,
    payloadAborted' = payloadAborted,
    lastOutcome' = lastOutcome,
  }

  // ----------------------------------------------------- behavior observations

  /// An instruction was rendered by the catch-all as raw program id + bytes.
  val RawFallbackRendered: bool =
    seen.exists(m => m.visualizer == UNKNOWN_VISUALIZER and m.status == "raw")

  /// The catch-all decoded an instruction using a caller-supplied IDL from the
  /// registry rather than falling back to raw bytes.
  val RegistryIdlDecoded: bool =
    seen.exists(m => m.visualizer == UNKNOWN_VISUALIZER and m.status == "decoded")

  /// A dedicated preset produced a decoded, named rendering.
  val PresetDecodedInstruction: bool =
    seen.exists(m => m.visualizer != UNKNOWN_VISUALIZER and m.status == "decoded")

  /// A preset could not parse the bytes but still produced a field, honouring
  /// the partial-rendering contract.
  val PresetRenderedUnparsable: bool =
    seen.exists(m => m.status == "unparsable")

  /// A visualizer returned Err, which aborts the whole transaction payload in
  /// the production build.
  val PayloadAborted: bool =
    seen.exists(m => m.status == "error")

  // ------------------------------------------------------------- invariants

  /// Every instruction the decoder reaches either contributes a payload field
  /// or aborts the payload; none is silently dropped.
  val everyInstructionAccountedFor: bool =
    instructionsProcessed == fieldsRendered + (if (payloadAborted) 1 else 0)

  /// The catch-all never fails, so it can never abort a payload.
  val catchAllNeverAborts: bool =
    seen.forall(m => m.visualizer != UNKNOWN_VISUALIZER or m.status != "error")

  /// A partial-rendering preset never aborts a payload.
  val partialRenderPresetNeverAborts: bool =
    seen.forall(m => m.visualizer != SPL_TOKEN_VISUALIZER or m.status != "error")

  /// Every OKX Router instruction is renderable: the signer gets a field for
  /// it and it never takes down the surrounding transaction payload.
  val okxRouterNeverAbortsPayload: bool =
    seen.forall(m => m.program != OKX_ROUTER_PROGRAM or m.status != "error")
}

/// Oracle main: replay entry point. Every nondet pick is branch-local and
/// named exactly as the argument the instrumented code logs.
module SolanaInstructionVisualizationOracle {
  import solana_instruction_visualization.*

  action init: bool = specInit

  action step: bool = any {
    beginTransaction,
    finishTransaction,
    {
      nondet programId = PROGRAM_IDS.oneOf()
      nondet dataLen = 0.to(MAX_DATA_LEN).oneOf()
      nondet discriminatorKnown = Bool.oneOf()
      nondet allAccountsResolved = Bool.oneOf()
      nondet accountCount = 0.to(MAX_ACCOUNTS).oneOf()
      nondet idlAccountCount = 0.to(MAX_ACCOUNTS).oneOf()
      nondet registryHasIdl = Bool.oneOf()
      visualizeInstruction(
        programId, dataLen, discriminatorKnown, allAccountsResolved,
        accountCount, idlAccountCount, registryHasIdl,
      )
    },
  }
}

module SolanaInstructionVisualizationTest {
  import solana_instruction_visualization.*

  action init: bool = specInit

  /// Happy path: a registered preset decodes an instruction and the payload
  /// carries one field.
  run splTokenDecodesTest =
    specInit
      .then(beginTransaction)
      .then(visualizeInstruction(SPL_TOKEN_PROGRAM, 9, true, true, 6, 4, false))
      .then(finishTransaction)
      .expect(and {
        PresetDecodedInstruction,
        lastOutcome.visualizer == SPL_TOKEN_VISUALIZER,
        lastOutcome.namedAccounts == 4,
        fieldsRendered == 1,
        not(payloadAborted),
      })

  /// A partial-rendering preset degrades unparsable bytes to a field instead
  /// of aborting the payload.
  run splTokenUnparsableStillRendersTest =
    specInit
      .then(beginTransaction)
      .then(visualizeInstruction(SPL_TOKEN_PROGRAM, 0, false, true, 3, 3, false))
      .then(finishTransaction)
      .expect(and {
        PresetRenderedUnparsable,
        fieldsRendered == 1,
        not(payloadAborted),
      })

  /// A program with no preset falls through to the catch-all and renders raw.
  run unregisteredProgramRendersRawTest =
    specInit
      .then(beginTransaction)
      .then(visualizeInstruction(UNREGISTERED_PROGRAM, 12, true, true, 5, 5, false))
      .then(finishTransaction)
      .expect(and {
        RawFallbackRendered,
        lastOutcome.visualizer == UNKNOWN_VISUALIZER,
        fieldsRendered == 1,
        not(payloadAborted),
      })

  /// The catch-all decodes when the caller supplied an IDL for the program.
  run catchAllUsesRegistryIdlTest =
    specInit
      .then(beginTransaction)
      .then(visualizeInstruction(UNREGISTERED_PROGRAM, 12, true, true, 5, 3, true))
      .then(finishTransaction)
      .expect(and {
        RegistryIdlDecoded,
        lastOutcome.namedAccounts == 3,
        not(payloadAborted),
      })

  /// An IDL preset that cannot decode the bytes aborts the whole payload: the
  /// field of the instruction before it is lost too.
  run idlPresetErrorAbortsPayloadTest =
    specInit
      .then(beginTransaction)
      .then(visualizeInstruction(SPL_TOKEN_PROGRAM, 9, true, true, 6, 4, false))
      .then(visualizeInstruction(METADAO_PROGRAM, 8, false, true, 6, 6, false))
      .then(finishTransaction)
      .expect(and {
        PayloadAborted,
        payloadAborted,
        instructionsProcessed == 2,
        fieldsRendered == 1,
      })

  /// OKX Router has no preset: its instructions reach the catch-all and render
  /// as raw bytes, so the signer sees an undecoded instruction but the payload
  /// always survives.
  run okxRouterInstructionTest =
    specInit
      .then(beginTransaction)
      .then(visualizeInstruction(OKX_ROUTER_PROGRAM, 12, true, true, 7, 5, false))
      .then(finishTransaction)
      .expect(and {
        lastOutcome.visualizer == UNKNOWN_VISUALIZER,
        lastOutcome.status == "raw",
        fieldsRendered == 1,
        not(payloadAborted),
        okxRouterNeverAbortsPayload,
      })

  /// The decoder is a stream: per-transaction counters reset, sticky behavior
  /// observations do not.
  run twoTransactionsTest =
    specInit
      .then(beginTransaction)
      .then(visualizeInstruction(SPL_TOKEN_PROGRAM, 9, true, true, 6, 4, false))
      .then(finishTransaction)
      .then(beginTransaction)
      .then(visualizeInstruction(UNREGISTERED_PROGRAM, 3, false, true, 2, 2, false))
      .then(finishTransaction)
      .expect(and {
        instructionsProcessed == 1,
        fieldsRendered == 1,
        PresetDecodedInstruction,
        RawFallbackRendered,
      })
}
