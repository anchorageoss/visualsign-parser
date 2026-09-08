// -*- mode: quint -*-
//
// Parse And Sign Pipeline -- the enclave parse endpoint.
//
// Models the per-request state machine of `Processor::process`
// (src/parser/app/src/service.rs:32) and `parse_with_registry`
// (src/parser/app/src/routes/parse.rs:36):
//
//   input demux (parse | health | missing input)
//     -> empty-payload guard
//     -> ProtoChain::try_from  (unknown enum code -> InvalidArgument)
//     -> build VisualSignOptions (decode_transfers = true, developer_config = None)
//     -> chain_conversion::proto_to_registry + registry.convert_transaction
//     -> UNCONDITIONAL SignablePayload::validate_charset
//     -> serde_json::to_string + ParsedTransactionPayload assembly
//     -> signing_digest_bytes -> sha256 -> P256 ephemeral sign
//
// WHAT THIS COVERS
//   * the error taxonomy of the endpoint (which stage rejects with which gRPC code)
//   * the charset gate as a property of the PIPELINE, not of any converter: the ONLY
//     path from a converted payload to the signing key runs through
//     `charset_validated`
//   * the derivation of `metadata_digest` (order-free) and `input_payload_digest`
//   * the borsh signing-bytes contract around `intermediate_output`
//     (`#[borsh(skip)]` + conditional append)
//
// WHAT THIS DOES NOT COVER
//   * chain-specific decoding (that is the per-chain converter crates)
//   * P256 / sha256 cryptography: hashing is modelled as injective, so a digest is
//     represented by its preimage and digest equality == preimage equality
//   * concurrency: the endpoint is modelled one request at a time, which matches
//     `block_in_place` + a per-request registry lookup
//   * the ephemeral-key fetch failing (`Processor::process` -> Internal). Every
//     deployment and every test has the key, so it is left out rather than
//     modelled as an always-false branch
//   * payload CONTENT: the rendered JSON is abstracted by the input it was rendered
//     from, and `intermediate_output` by whether it is empty. Presence is what the
//     digest contract turns on, and it is what the real code can report
//   * the actual borsh byte layout: the four-field derived encoding is modelled as
//     a record and the appended `borsh(Vec<u8>)` (u32-LE length prefix + bytes) as a
//     boolean "were the intermediate bytes appended". Both encodings stay
//     distinguishable, which is all the backward-compatibility property needs.
//
// REPLAY SURFACE
//   Every logged argument is something the real code can observe at the site where
//   the transition is reported: the proto chain name, whether `chain_metadata` was
//   supplied, `include_intermediate_output`, and whether the converter returned any
//   intermediate bytes. Which branch a request took is carried by WHICH action is
//   logged, never by an argument -- the code does not know in advance whether its
//   payload will clear the charset gate.
module parse_sign_pipeline {

  // ---------------------------------------------------------------------------
  // Chains
  // ---------------------------------------------------------------------------

  /// The proto `Chain` enum, by its generated `as_str_name()` spelling
  /// (proto/parser/parser.proto:14) -- this is exactly what the instrumented code
  /// logs. A wire value OUTSIDE the enum is not in this set: `try_from` fails on it,
  /// so it is reported by `reject_unknown_chain`, which carries no chain argument.
  pure val CHAIN_NAMES: Set[str] = Set(
    "CHAIN_UNSPECIFIED",
    "CHAIN_BITCOIN",
    "CHAIN_ETHEREUM",
    "CHAIN_SOLANA",
    "CHAIN_SUI",
    "CHAIN_TRON",
    "CHAIN_CUSTOM",
  )

  /// `chain_conversion::proto_to_registry` (src/parser/app/src/chain_conversion.rs:6),
  /// by `visualsign::registry::Chain::as_str`. Everything unmapped collapses into the
  /// `Custom("custom_unknown")` slot.
  pure def protoToRegistry(chain: str): str =
    if (chain == "CHAIN_SOLANA") "Solana"
    else if (chain == "CHAIN_ETHEREUM") "Ethereum"
    else if (chain == "CHAIN_SUI") "Sui"
    else if (chain == "CHAIN_TRON") "Tron"
    else if (chain == "CHAIN_UNSPECIFIED") "Unspecified"
    else "custom_unknown"

  /// The chains `create_registry` registers with the default feature set
  /// (src/parser/app/src/registry.rs:12). `custom_unknown` is deliberately absent:
  /// it is the registry-miss slot, and a miss is a conversion error.
  pure val REGISTERED_CHAINS: Set[str] =
    Set("Unspecified", "Solana", "Ethereum", "Sui", "Tron")

  pure def hasConverter(registryChain: str): bool =
    REGISTERED_CHAINS.contains(registryChain)

  // ---------------------------------------------------------------------------
  // Chain metadata and digests
  // ---------------------------------------------------------------------------

  /// Caller-supplied `ParseRequest.chain_metadata`, reduced to the ABI-mapping keys.
  /// `entries` is a LIST so the model can distinguish two callers that supplied the
  /// same entries in different insertion orders -- the exact difference
  /// `metadata_digest` must be insensitive to.
  ///
  /// NOTE for instrumentation: the proto map is a `BTreeMap` (via
  /// `tonic_build::Builder::btree_map(["."])`), so by the time the pipeline sees it
  /// the insertion order is already gone. The order is therefore a free choice in
  /// the model and is never logged.
  type Metadata = { present: bool, entries: List[str] }

  pure val NO_METADATA: Metadata = { present: false, entries: List() }

  pure def suppliedMetadata(hasMetadata: bool, metadataKeys: List[str]): Metadata =
    if (hasMetadata) { present: true, entries: metadataKeys } else NO_METADATA

  /// Insertion order collapsed away: this is what borsh sees once the proto map is
  /// a `BTreeMap`.
  pure def entrySet(abiKeys: List[str]): Set[str] =
    abiKeys.foldl(Set(), (acc, e) => acc.union(Set(e)))

  /// A SHA-256 digest, represented by its preimage. `domain` separates the preimage
  /// spaces so an input digest can never accidentally equal a metadata digest.
  type Digest = { domain: str, text: str, entries: Set[str] }

  /// `sha256(unsigned_payload_bytes)` (parse.rs:100).
  pure def inputDigestOf(payload: str): Digest =
    { domain: "input", text: payload, entries: Set() }

  /// `sha256(borsh(metadata))`, or `sha256(vec![])` when no metadata was supplied
  /// (parse.rs:92-101). Absent metadata gets its own domain because
  /// `borsh(ChainMetadata { .. })` is never the empty byte string.
  pure def metadataDigestOf(md: Metadata): Digest =
    if (md.present) { domain: "metadata", text: "", entries: entrySet(md.entries) }
    else { domain: "empty", text: "", entries: Set() }

  pure def sameMetadataContent(x: Metadata, y: Metadata): bool =
    x.present == y.present and entrySet(x.entries) == entrySet(y.entries)

  /// The same entries as they would have arrived in the opposite insertion order.
  pure def reversedEntries(abiKeys: List[str]): List[str] =
    abiKeys.foldl(List(), (acc, e) => List(e).concat(acc))

  // ---------------------------------------------------------------------------
  // Options
  // ---------------------------------------------------------------------------

  /// `VisualSignOptions` as built on the production signing path (parse.rs:49).
  type Options = {
    decodeTransfers: bool,
    developerConfigEnabled: bool,
    includeIntermediateOutput: bool,
  }

  pure val NO_OPTIONS: Options = {
    decodeTransfers: false,
    developerConfigEnabled: false,
    includeIntermediateOutput: false,
  }

  /// The endpoint hard-codes `decode_transfers: true` and `developer_config: None`
  /// -- the latter is what makes already-signed transactions unacceptable here.
  pure def productionOptions(includeIntermediate: bool): Options = {
    decodeTransfers: true,
    developerConfigEnabled: false,
    includeIntermediateOutput: includeIntermediate,
  }

  // ---------------------------------------------------------------------------
  // The transported payload and the bytes that get signed
  // ---------------------------------------------------------------------------

  /// `ParsedTransactionPayload` (proto/parser/parser.proto:81), the message that
  /// actually travels back to the caller. `intermediateOutput` records whether any
  /// intermediate bytes are carried, which is what the digest contract turns on.
  type TransportPayload = {
    parsedPayload: str,
    inputPayloadDigest: Digest,
    metadataDigest: Digest,
    signablePayload: str,
    intermediateOutput: bool,
  }

  /// The four fields the DERIVED borsh encoding covers. `intermediate_output` is
  /// `#[borsh(skip)]`, so it is absent here by construction.
  type BorshFields = {
    parsedPayload: str,
    inputPayloadDigest: Digest,
    metadataDigest: Digest,
    signablePayload: str,
  }

  /// The byte string the ephemeral key signs over: the derived four-field encoding,
  /// plus the borsh `Vec<u8>` encoding of `intermediate_output` appended only when
  /// that field is non-empty.
  type SignedBytes = { borshFields: BorshFields, appendedIntermediate: bool }

  pure def transportPayloadOf(payload: str, md: Metadata, hasIntermediate: bool): TransportPayload = {
    // The rendered JSON is abstracted by the input it was rendered from.
    parsedPayload: payload,
    inputPayloadDigest: inputDigestOf(payload),
    metadataDigest: metadataDigestOf(md),
    // Legacy duplicate, kept until clients migrate (parse.rs:103).
    signablePayload: payload,
    intermediateOutput: hasIntermediate,
  }

  pure def borshFieldsOf(p: TransportPayload): BorshFields = {
    parsedPayload: p.parsedPayload,
    inputPayloadDigest: p.inputPayloadDigest,
    metadataDigest: p.metadataDigest,
    signablePayload: p.signablePayload,
  }

  /// `borsh::to_vec(payload)` alone -- the pre-feature four-field encoding an old
  /// verifier reproduces.
  pure def legacyBorshBytes(p: TransportPayload): SignedBytes =
    { borshFields: borshFieldsOf(p), appendedIntermediate: false }

  /// `signing_digest_bytes` (parse.rs:145). Derived ENTIRELY from `p`, so the bytes
  /// signed and the bytes transported cannot diverge.
  pure def signingDigestBytes(p: TransportPayload): SignedBytes =
    { borshFields: borshFieldsOf(p), appendedIntermediate: p.intermediateOutput }

  pure val EPHEMERAL_PUBKEY: str = "ephemeral-p256-pubkey"

  /// `Signature` (parse.rs:115). `message` is the digest over `SignedBytes`;
  /// hashing is injective in this model, so the preimage stands in for it.
  type Signature = { message: SignedBytes, publicKey: str, scheme: str }

  /// One completed signing, appended to the ledger. Keeping the request, the
  /// options, the charset verdict, the transported payload AND the signed bytes side
  /// by side is what lets the invariants below relate them.
  type SignedRecord = {
    chain: str,
    payload: str,
    metadata: Metadata,
    options: Options,
    charsetValidated: bool,
    transported: TransportPayload,
    signedOver: SignedBytes,
    signature: Signature,
  }

  // ---------------------------------------------------------------------------
  // Outcomes
  // ---------------------------------------------------------------------------

  /// The `google.rpc.Code` the endpoint answers with.
  type StatusCode = CodeNone | CodeOk | CodeInvalidArgument | CodeInternal

  /// The response the endpoint produced, plus the stage that produced it.
  type Outcome = { code: StatusCode, stage: str }

  pure val NO_OUTCOME: Outcome = { code: CodeNone, stage: "none" }
  pure val OUTCOME_SIGNED: Outcome = { code: CodeOk, stage: "signed" }
  pure val OUTCOME_HEALTH: Outcome = { code: CodeOk, stage: "health" }

  // ---------------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------------

  /// Where a request has got to inside `parse_with_registry`.
  type Phase =
      PhaseIdle       // no request in flight; `outcome` holds the last response
    | PhaseDispatched // pre-checks passed, options built, handed to the registry
    | PhaseConverted  // the converter returned a payload; the charset gate is next
    | PhaseValidated  // the charset gate PASSED; assembly and signing are next

  /// The in-flight request. Fields other than `phase`/`outcome` are only meaningful
  /// once a request has been dispatched; at `PhaseIdle` they hold the last request
  /// so the observations can say WHICH request produced the outcome.
  type Pipeline = {
    phase: Phase,
    chain: str,
    payload: str,
    metadata: Metadata,
    options: Options,
    hasIntermediate: bool,
    charsetValidated: bool,
    outcome: Outcome,
  }

  pure val IDLE_PIPELINE: Pipeline = {
    phase: PhaseIdle,
    chain: "",
    payload: "",
    metadata: NO_METADATA,
    options: NO_OPTIONS,
    hasIntermediate: false,
    charsetValidated: false,
    outcome: NO_OUTCOME,
  }

  var pipeline: Pipeline

  /// Every signing that happened, oldest first. The history is state because the
  /// interesting properties (order-independent metadata digests, "nothing unvalidated
  /// was EVER signed") relate signings to each other.
  var ledger: List[SignedRecord]

  // ---------------------------------------------------------------------------
  // Free (unlogged) choices
  // ---------------------------------------------------------------------------

  /// Abstract non-empty `unsigned_payload` values. Two are enough to state that the
  /// input digest tracks the payload; the real bytes are never logged.
  pure val PAYLOADS: Set[str] = Set("tx-a", "tx-b")

  /// The same two ABI-mapping keys in either insertion order. The pair is what makes
  /// the `metadata_digest` determinism property non-vacuous; the order is not
  /// observable from the code (see `Metadata`).
  pure val META_ORDERS: Set[List[str]] = Set(
    List("0xaaaa", "0xbbbb"),
    List("0xbbbb", "0xaaaa"),
  )

  pure val BOOLS: Set[bool] = Set(false, true)

  // ---------------------------------------------------------------------------
  // Actions
  // ---------------------------------------------------------------------------

  action init: bool = all {
    pipeline' = IDLE_PIPELINE,
    ledger' = List(),
  }

  /// Finish a request without signing: record the response and go back to idle,
  /// keeping the request fields around for inspection.
  action settle(result: Outcome): bool = all {
    pipeline' = { ...pipeline, phase: PhaseIdle, outcome: result },
    ledger' = ledger,
  }

  /// The `HealthRequest` arm of the `oneof` answers 200 without entering the parse
  /// pipeline (service.rs:82).
  action health_check: bool = all {
    pipeline.phase == PhaseIdle,
    pipeline' = { ...IDLE_PIPELINE, outcome: OUTCOME_HEALTH },
    ledger' = ledger,
  }

  /// `request.input` was `None` -> InvalidArgument (service.rs:52).
  action reject_missing_input: bool = all {
    pipeline.phase == PhaseIdle,
    pipeline' = { ...IDLE_PIPELINE, outcome: { code: CodeInvalidArgument, stage: "missing_input" } },
    ledger' = ledger,
  }

  /// First guard of `parse_with_registry` (parse.rs:42): an empty `unsigned_payload`
  /// is rejected with InvalidArgument before anything else runs.
  action reject_empty_payload: bool = all {
    pipeline.phase == PhaseIdle,
    pipeline' = { ...IDLE_PIPELINE, outcome: { code: CodeInvalidArgument, stage: "empty_payload" } },
    ledger' = ledger,
  }

  /// `ProtoChain::try_from(parse_request.chain)` failed (parse.rs:56) -> "invalid
  /// chain", InvalidArgument. No chain argument: the value is outside the enum, so
  /// there is nothing in `CHAIN_NAMES` to name it with.
  action reject_unknown_chain: bool = all {
    pipeline.phase == PhaseIdle,
    pipeline' = { ...IDLE_PIPELINE, outcome: { code: CodeInvalidArgument, stage: "unknown_chain" } },
    ledger' = ledger,
  }

  /// Pre-checks passed: build the production `VisualSignOptions`, map the proto chain
  /// onto the registry chain, and hand the payload to `registry.convert_transaction`
  /// (parse.rs:60).
  ///
  /// `chain`, `hasMetadata` and `includeIntermediate` are logged by the real code at
  /// this site; `payload` and `metadataKeys` are free choices (see above).
  action dispatch_to_converter(chain: str, payload: str, hasMetadata: bool,
                               metadataKeys: List[str], includeIntermediate: bool): bool = all {
    pipeline.phase == PhaseIdle,
    pipeline' = {
      phase: PhaseDispatched,
      chain: chain,
      payload: payload,
      metadata: suppliedMetadata(hasMetadata, metadataKeys),
      options: productionOptions(includeIntermediate),
      hasIntermediate: false,
      charsetValidated: false,
      outcome: NO_OUTCOME,
    },
    ledger' = ledger,
  }

  /// `registry.convert_transaction` returned `Err`: either no converter is registered
  /// for the resolved chain, or the converter refused the payload. Both surface as
  /// InvalidArgument (parse.rs:62).
  action conversion_failed: bool = all {
    pipeline.phase == PhaseDispatched,
    settle({ code: CodeInvalidArgument, stage: "conversion" }),
  }

  /// The converter produced a `SignablePayload`. NOTHING has validated it yet as far
  /// as this pipeline is concerned -- the converter may have skipped its own
  /// `validate_charset` (e.g. Ethereum's override).
  ///
  /// Guarded twice, and both guards are real contracts: a chain with no registered
  /// converter cannot convert successfully, and `intermediate_output` is only
  /// populated when the caller asked for it (proto/parser/parser.proto:93).
  action conversion_succeeded(hasIntermediate: bool): bool = all {
    pipeline.phase == PhaseDispatched,
    hasConverter(protoToRegistry(pipeline.chain)),
    hasIntermediate implies pipeline.options.includeIntermediateOutput,
    pipeline' = { ...pipeline, phase: PhaseConverted, hasIntermediate: hasIntermediate },
    ledger' = ledger,
  }

  /// The unconditional charset gate passed (parse.rs:78). This is the ONLY route from
  /// `PhaseConverted` to signing, which is how the pipeline enforces the gate for
  /// every converter.
  action charset_validated: bool = all {
    pipeline.phase == PhaseConverted,
    pipeline' = { ...pipeline, phase: PhaseValidated, charsetValidated: true },
    ledger' = ledger,
  }

  /// `validate_charset` returned `ValidationError` -> InvalidArgument, and the payload
  /// never reaches the signing key, no matter which converter produced it.
  action reject_charset_violation: bool = all {
    pipeline.phase == PhaseConverted,
    settle({ code: CodeInvalidArgument, stage: "charset" }),
  }

  /// A non-`ValidationError` out of `validate_charset`, a failing
  /// `serde_json::to_string`, or a failing `sign`: server-side bugs, so `Internal`,
  /// not InvalidArgument. Reachable both before and after the charset gate, matching
  /// where those calls sit in the code.
  action reject_internal_serialization_failure: bool = all {
    any {
      pipeline.phase == PhaseConverted,
      pipeline.phase == PhaseValidated,
    },
    settle({ code: CodeInternal, stage: "internal" }),
  }

  /// Assemble `ParsedTransactionPayload`, derive the signing bytes from that same
  /// value, hash, and sign with the ephemeral P256 key (parse.rs:98-120).
  action sign_parsed_payload: bool = all {
    pipeline.phase == PhaseValidated,
    pipeline' = { ...pipeline, phase: PhaseIdle, outcome: OUTCOME_SIGNED },
    ledger' = ledger.append({
      chain: pipeline.chain,
      payload: pipeline.payload,
      metadata: pipeline.metadata,
      options: pipeline.options,
      charsetValidated: pipeline.charsetValidated,
      transported: transportPayloadOf(pipeline.payload, pipeline.metadata, pipeline.hasIntermediate),
      signedOver: signingDigestBytes(
        transportPayloadOf(pipeline.payload, pipeline.metadata, pipeline.hasIntermediate)
      ),
      signature: {
        message: signingDigestBytes(
          transportPayloadOf(pipeline.payload, pipeline.metadata, pipeline.hasIntermediate)
        ),
        publicKey: EPHEMERAL_PUBKEY,
        scheme: "TURNKEY_P256_EPHEMERAL_KEY",
      },
    }),
  }

  action step: bool = any {
    health_check,
    reject_missing_input,
    reject_empty_payload,
    reject_unknown_chain,
    {
      nondet chain = CHAIN_NAMES.oneOf()
      nondet payload = PAYLOADS.oneOf()
      nondet hasMetadata = BOOLS.oneOf()
      nondet metadataKeys = META_ORDERS.oneOf()
      nondet includeIntermediate = BOOLS.oneOf()
      dispatch_to_converter(chain, payload, hasMetadata, metadataKeys, includeIntermediate)
    },
    conversion_failed,
    {
      nondet hasIntermediate = BOOLS.oneOf()
      conversion_succeeded(hasIntermediate)
    },
    charset_validated,
    reject_charset_violation,
    reject_internal_serialization_failure,
    sign_parsed_payload,
  }

  // ---------------------------------------------------------------------------
  // Observations -- the behaviors worth measuring test coverage for
  // ---------------------------------------------------------------------------

  /// A parse request ran the whole pipeline and a signed response was returned.
  val signed_response_returned: bool = pipeline.outcome == OUTCOME_SIGNED

  /// A converted payload failed charset validation and was rejected with
  /// InvalidArgument -- the pipeline's unconditional check caught it before signing,
  /// even when the converter skipped its own validation.
  val charset_violation_rejected: bool =
    pipeline.outcome == { code: CodeInvalidArgument, stage: "charset" }

  /// An empty `unsigned_payload` was rejected with InvalidArgument.
  val empty_payload_rejected: bool =
    pipeline.outcome == { code: CodeInvalidArgument, stage: "empty_payload" }

  /// A `chain` value outside the proto enum was rejected with InvalidArgument.
  val unknown_chain_rejected: bool =
    pipeline.outcome == { code: CodeInvalidArgument, stage: "unknown_chain" }

  /// A registered converter refused to decode the transaction, and the converter's
  /// error surfaced as InvalidArgument.
  val unparseable_transaction_rejected: bool =
    and {
      pipeline.outcome.stage == "conversion",
      hasConverter(protoToRegistry(pipeline.chain)),
    }

  /// A chain that resolves to the unregistered `custom_unknown` slot was rejected:
  /// the registry miss surfaces as InvalidArgument, not a panic.
  val unregistered_chain_rejected: bool =
    and {
      pipeline.outcome.stage == "conversion",
      not(hasConverter(protoToRegistry(pipeline.chain))),
    }

  /// Something was signed whose `intermediate_output` was empty, so the signed bytes
  /// are exactly the legacy four-field borsh encoding.
  val signed_with_empty_intermediate_output: bool =
    ledger.foldl(false, (seen, r) => seen or not(r.transported.intermediateOutput))

  /// Something was signed whose `intermediate_output` was non-empty, so its digest
  /// differs from the legacy encoding.
  val signed_with_intermediate_output: bool =
    ledger.foldl(false, (seen, r) => seen or r.transported.intermediateOutput)

  /// A request carrying MORE THAN ONE ABI mapping -- so insertion order could have
  /// mattered -- was signed, and its `metadata_digest` is the same one the opposite
  /// insertion order of those entries would have produced.
  ///
  /// Stated over a single signing on purpose: the code never sees the caller's
  /// insertion order (the proto map is a `BTreeMap`), so no pair of real requests can
  /// exhibit the difference. The cross-signing form of the property is the invariant
  /// `metadata_digest_independent_of_insertion_order`.
  val metadata_digest_agreed_across_insertion_orders: bool =
    ledger.foldl(false, (seen, r) => seen or and {
      r.metadata.present,
      r.metadata.entries.length() > 1,
      r.transported.metadataDigest
        == metadataDigestOf({ ...r.metadata, entries: reversedEntries(r.metadata.entries) }),
    })

  /// A request with no `chain_metadata` was signed against the empty-metadata digest.
  val signed_without_chain_metadata: bool =
    ledger.foldl(false, (seen, r) => seen or not(r.metadata.present))

  /// A request carrying `chain_metadata` was signed, so caller-supplied metadata is
  /// bound into the digest.
  val signed_with_chain_metadata: bool =
    ledger.foldl(false, (seen, r) => seen or r.metadata.present)

  /// A health request was answered 200 without entering the parse pipeline.
  val health_check_answered: bool = pipeline.outcome == OUTCOME_HEALTH

  // ---------------------------------------------------------------------------
  // Invariants
  // ---------------------------------------------------------------------------

  /// THE headline safety property: nothing is signed that did not clear the charset
  /// gate, regardless of which converter produced it or whether that converter ran
  /// its own validation. `charset_validated` is the only way into `PhaseValidated`,
  /// and `sign_parsed_payload` is only enabled there.
  val never_signs_without_charset_validation: bool =
    ledger.foldl(true, (ok, r) => ok and r.charsetValidated)

  /// The signed digest is derived only from the `ParsedTransactionPayload` that is
  /// transported, so signed bytes and transported bytes can never diverge.
  val signed_bytes_derived_from_transported_payload: bool =
    ledger.foldl(true, (ok, r) => and {
      ok,
      r.signedOver == signingDigestBytes(r.transported),
      r.signature.message == r.signedOver,
    })

  /// Backward compatibility: an empty `intermediate_output` yields byte-identical
  /// bytes to the legacy four-field borsh encoding, so old verifiers keep verifying.
  val empty_intermediate_matches_legacy_encoding: bool =
    ledger.foldl(true, (ok, r) => ok and (
      not(r.transported.intermediateOutput)
        implies r.signedOver == legacyBorshBytes(r.transported)
    ))

  /// A non-empty `intermediate_output` changes the signed digest, so an
  /// intermediate-unaware consumer rejects rather than mis-verifies.
  val non_empty_intermediate_changes_signed_bytes: bool =
    ledger.foldl(true, (ok, r) => ok and (
      r.transported.intermediateOutput
        implies r.signedOver != legacyBorshBytes(r.transported)
    ))

  /// Identical chain metadata always produces the same `metadata_digest`, regardless
  /// of map insertion order.
  val metadata_digest_independent_of_insertion_order: bool =
    ledger.indices().forall(i => ledger.indices().forall(j => {
      val a = ledger.nth(i)
      val b = ledger.nth(j)
      sameMetadataContent(a.metadata, b.metadata)
        implies a.transported.metadataDigest == b.transported.metadataDigest
    }))

  /// Nothing reaches the signing key without a registered converter having accepted
  /// it, and never with an empty payload.
  val only_registered_chains_reach_signing: bool =
    ledger.foldl(true, (ok, r) => and {
      ok,
      hasConverter(protoToRegistry(r.chain)),
      r.payload != "",
    })

  /// The production endpoint passes `developer_config: None` and
  /// `decode_transfers: true`, which is what makes already-signed transactions
  /// unacceptable here.
  val production_options_never_enable_developer_config: bool =
    ledger.foldl(true, (ok, r) => and {
      ok,
      not(r.options.developerConfigEnabled),
      r.options.decodeTransfers,
    })

  /// The legacy `signable_payload` field always mirrors `parsed_payload`, so clients
  /// reading either see the same JSON.
  val legacy_signable_payload_mirrors_parsed_payload: bool =
    ledger.foldl(true, (ok, r) =>
      ok and r.transported.parsedPayload == r.transported.signablePayload)

  /// `intermediate_output` is only ever populated when the caller asked for it.
  val intermediate_output_only_when_requested: bool =
    ledger.foldl(true, (ok, r) => ok and (
      r.transported.intermediateOutput implies r.options.includeIntermediateOutput
    ))

  /// `input_payload_digest` always covers exactly the bytes the caller submitted.
  val input_digest_binds_the_request_payload: bool =
    ledger.foldl(true, (ok, r) =>
      ok and r.transported.inputPayloadDigest == inputDigestOf(r.payload))
}
