// -*- mode: quint -*-
//
// Deterministic Serialization -- the cross-cutting guarantee that the same
// logical transaction always produces the same bytes.
//
// This component is not a state machine in the product; it is the property that
// relates `parse-sign-pipeline`, `signable-payload-validation` and
// `solana-instruction-pipeline`. It is therefore modelled as an ENCODING LEDGER:
// every action performs ONE real serialization, records the logical content that
// went in beside the bytes that came out, and the invariants relate those records
// pairwise. "Deterministic" is then a statement over two build orders of the same
// content, exactly as the specHints ask for.
//
// THE FOUR MECHANISMS MODELLED
//   1. Alphabetical serde field order -- `DeterministicOrdering` and the custom
//      `impl Serialize for SignablePayloadField`, which pushes entries into a
//      `HashMap` and then collects into a `BTreeMap` before emitting
//      (src/visualsign/src/lib.rs:20-108, :324-397).
//   2. `tonic_build::configure().btree_map(["."])` -- every proto `map<..>` field
//      is a `BTreeMap`, which is what makes `borsh(chain_metadata)` insertion-order
//      independent (src/codegen/src/main.rs:40).
//   3. `canonical_args_json` / `canonicalize_value` / `canonicalize_map` -- the
//      RECURSIVE re-key of the `serde_json::Value` tree, needed because
//      `serde_json`'s `preserve_order` feature is enabled transitively in this
//      workspace, so a top-level-only sort would let nested insertion order leak
//      into the bytes (src/chain_parsers/visualsign-solana/src/intermediate.rs:189-231).
//   4. `#[borsh(skip)]` on `intermediate_output` plus the explicit append in
//      `signing_digest_bytes` -- the legacy four-field encoding stays byte-stable
//      while the field is still bound into the signed digest
//      (src/codegen/src/main.rs:25,67-70 and src/parser/app/src/routes/parse.rs:145).
//
// HOW BYTES ARE MODELLED
//   Emission ORDER is the only thing that can make two encodings of the same
//   content differ, so a serialized map is modelled by the shape of its emission:
//   a `Set` when the order is determined by CONTENT alone (a `BTreeMap` re-key, or
//   the sorted-insertion walk of `canonicalize_map`) and a `List` when it follows
//   INSERTION order (a `HashMap`, or `serde_json`'s `preserve_order` `IndexMap`).
//   Two `Set`-emissions of the same content are the same value by construction --
//   which is precisely the guarantee the code buys with `BTreeMap`. The
//   insertion-order constructors are the counterfactual: they are what the same
//   sites emitted (or would emit) WITHOUT those mechanisms, and they are what makes
//   the determinism invariants discriminating rather than vacuous.
//
// WHAT THIS COVERS
//   * order-independence of every map that reaches a hash (proto maps, named
//     accounts, payload field maps, nested JSON objects at every level)
//   * order-DEPENDENCE of every vector that reaches a hash (`account_keys`, JSON
//     arrays): element order there is content, not build order
//   * observability: distinct logical content, and any bump of the intermediate
//     output's schema shape, always changes the bytes
//   * the borsh-skip/append contract around `intermediate_output`
//
// WHAT THIS DOES NOT COVER
//   * sha256/borsh byte layout: hashing is modelled as injective, so a digest is
//     represented by its preimage and digest equality == preimage equality
//   * JSON nesting is bounded to depth 2 (an object whose members are scalars,
//     one-level objects, or arrays of one-level objects) -- Quint has no recursion,
//     and depth 2 is exactly what distinguishes a recursive canonicalization from a
//     top-level-only sort
//   * the chain-specific decoding that PRODUCES the content being encoded (that is
//     the per-chain converter components)
module deterministic_serialization {

  // Order-free views of the lists this spec builds. These are deliberately
  // MONOMORPHIC one-per-element-type folds rather than one polymorphic helper:
  // the evaluator cannot re-enter a user-defined operator, and the JSON walk below
  // needs an order-free view at two nesting levels at once.

  /// The entries of a nested JSON object, order thrown away (a `BTreeMap` re-key).
  pure def leafSet(l: List[Leaf]): Set[Leaf] =
    l.foldl(Set(), (s, e) => s.union(Set(e)))

  /// `abi_mappings` entries, order thrown away (the proto map is a `BTreeMap`).
  pure def metaSet(l: List[MetaEntry]): Set[MetaEntry] =
    l.foldl(Set(), (s, e) => s.union(Set(e)))

  /// A payload field's key/value entries, order thrown away (`HashMap` -> `BTreeMap`).
  pure def fieldEntrySet(l: List[FieldEntry]): Set[FieldEntry] =
    l.foldl(Set(), (s, e) => s.union(Set(e)))

  /// `named_accounts` bindings, order thrown away (`HashMap` -> `BTreeMap`).
  pure def bindingSet(l: List[Binding]): Set[Binding] =
    l.foldl(Set(), (s, e) => s.union(Set(e)))

  pure def strSet(l: List[str]): Set[str] =
    l.foldl(Set(), (s, e) => s.union(Set(e)))

  // ===========================================================================
  // 1. A bounded JSON tree -- `program_call_args`
  // ===========================================================================

  /// One `"key": <scalar>` pair of a nested JSON object.
  type Leaf = { name: str, num: int }

  /// A JSON value, bounded to nesting depth 2: a scalar, a one-level object, or an
  /// array of one-level objects. Arrays are LISTS on purpose -- JSON array order is
  /// content, and `canonicalize_value` traverses arrays element-wise without
  /// reordering them.
  type Val =
      VNum(int)
    | VObj(List[Leaf])
    | VArr(List[List[Leaf]])

  /// One member of the top-level args object, in the order the builder inserted it.
  type Member = { name: str, value: Val }
  type JsonBuild = List[Member]

  /// How a JSON object's keys came out. `OSorted` = emitted in an order determined
  /// by content alone; `OInsertion` = emitted in insertion order, which is what
  /// `serde_json` does for a nested `Map` under the `preserve_order` feature.
  type ObjBytes =
      OSorted(Set[Leaf])
    | OInsertion(List[Leaf])

  type ValBytes =
      BNum(int)
    | BObj(ObjBytes)
    | BArr(List[ObjBytes])

  type MemberBytes = { name: str, value: ValBytes }

  /// The serialized args string.
  type JsonBytes =
      JSorted(Set[MemberBytes])
    | JInsertion(List[MemberBytes])

  /// Was this object emitted in an order determined by content alone?
  pure def objIsContentOrdered(o: ObjBytes): bool =
    match o {
      | OSorted(_)    => true
      | OInsertion(_) => false
    }

  pure def valIsContentOrdered(v: ValBytes): bool =
    match v {
      | BNum(_) => true
      | BObj(o) => objIsContentOrdered(o)
      | BArr(a) => a.foldl(true, (ok, o) => ok and objIsContentOrdered(o))
    }

  /// True when EVERY nesting level of the emitted JSON is content-ordered -- the
  /// thing `canonicalize_value`'s recursion buys over a top-level-only sort.
  pure def jsonIsContentOrdered(j: JsonBytes): bool =
    match j {
      | JSorted(members) => members.forall(m => valIsContentOrdered(m.value))
      | JInsertion(_)    => false
    }

  /// `canonicalize_value` (intermediate.rs:209): objects re-keyed into sorted order
  /// at EVERY nesting level, arrays traversed element-wise, scalars unchanged.
  pure def canonicalizeValue(v: Val): ValBytes =
    match v {
      | VNum(n)  => BNum(n)
      | VObj(o)  => BObj(OSorted(leafSet(o)))
      | VArr(a)  => BArr(a.foldl(List(), (acc, o) => acc.append(OSorted(leafSet(o)))))
    }

  /// `canonical_args_json` + `canonicalize_map` (intermediate.rs:189-231).
  pure def canonicalArgsJson(b: JsonBuild): JsonBytes =
    JSorted(b.foldl(Set(), (acc, m) =>
      acc.union(Set({ name: m.name, value: canonicalizeValue(m.value) }))))

  /// The counterfactual the recursive walk exists to prevent: alphabetize only the
  /// top-level map and let `preserve_order` emit nested objects in insertion order.
  /// Never produced by an action -- it is only used to state that the recursive
  /// walk is doing real work.
  pure def topLevelSortedOnly(b: JsonBuild): JsonBytes =
    JSorted(b.foldl(Set(), (acc, m) =>
      acc.union(Set({ name: m.name, value: insertionValue(m.value) }))))

  pure def insertionValue(v: Val): ValBytes =
    match v {
      | VNum(n)  => BNum(n)
      | VObj(o)  => BObj(OInsertion(o))
      | VArr(a)  => BArr(a.foldl(List(), (acc, o) => acc.append(OInsertion(o))))
    }

  /// The logical content of an args object: order-free at every OBJECT level, order
  /// preserving inside arrays. Two builds with equal content are what the component
  /// calls "two structurally identical inputs".
  type ValContent =
      CNum(int)
    | CObj(Set[Leaf])
    | CArr(List[Set[Leaf]])

  type MemberContent = { name: str, value: ValContent }
  type JsonContent = Set[MemberContent]

  pure def valueContent(v: Val): ValContent =
    match v {
      | VNum(n)  => CNum(n)
      | VObj(o)  => CObj(leafSet(o))
      | VArr(a)  => CArr(a.foldl(List(), (acc, o) => acc.append(leafSet(o))))
    }

  pure def argsContent(b: JsonBuild): JsonContent =
    b.foldl(Set(), (acc, m) =>
      acc.union(Set({ name: m.name, value: valueContent(m.value) })))

  // ===========================================================================
  // 2. Proto maps -- `ChainMetadata.abi_mappings`
  // ===========================================================================

  /// One entry of `EthereumMetadata.abi_mappings` (contract address -> ABI).
  type MetaEntry = { addr: str, abi: str }

  /// The borsh encoding of a proto `map<string, Abi>` field.
  type MapBytes =
      MSorted(Set[MetaEntry])
    | MInsertion(List[MetaEntry])

  /// `borsh::to_vec(&chain_metadata)` (parse.rs:93). codegen forces `BTreeMap` for
  /// every proto map (`btree_map(["."])`, codegen/src/main.rs:40), so borsh iterates
  /// in key order and the bytes are a function of content alone.
  pure def borshChainMetadata(entries: List[MetaEntry]): MapBytes =
    MSorted(metaSet(entries))

  /// What the same field encoded to before `btree_map(["."])`, when prost generated
  /// a `HashMap`: iteration order, and hence bytes, followed insertion.
  pure def hashMapChainMetadata(entries: List[MetaEntry]): MapBytes =
    MInsertion(entries)

  pure def mapIsContentOrdered(m: MapBytes): bool =
    match m {
      | MSorted(_)    => true
      | MInsertion(_) => false
    }

  // ===========================================================================
  // 3. Alphabetical serde field order -- `SignablePayloadField`
  // ===========================================================================

  /// The JSON keys a serialized `SignablePayloadField` can carry. Quint has no
  /// string comparison, so alphabetical order is carried explicitly by `keyRank`.
  type FieldKey =
      KeyAmountV2
    | KeyFallbackText
    | KeyLabel
    | KeyText
    | KeyKind        // the "Type" tag key; `type` is a Quint reserved word

  /// Alphabetical rank of each key: "AmountV2" < "FallbackText" < "Label" < "Text"
  /// < "Type". This is the order `get_expected_fields`'s `sort()` and the
  /// `BTreeMap` collect both produce (lib.rs:325, :348).
  pure def keyRank(k: FieldKey): int =
    match k {
      | KeyAmountV2     => 0
      | KeyFallbackText => 1
      | KeyLabel        => 2
      | KeyText         => 3
      | KeyKind         => 4
    }

  type FieldEntry = { key: FieldKey, text: str }

  /// Which `SignablePayloadField` variant is being serialized.
  type FieldVariant = VariantText | VariantAmountV2

  /// `get_expected_fields` (lib.rs:328): the three common keys plus the variant's
  /// own payload key.
  pure def expectedKeys(v: FieldVariant): Set[FieldKey] =
    match v {
      | VariantText      => Set(KeyFallbackText, KeyLabel, KeyKind, KeyText)
      | VariantAmountV2  => Set(KeyFallbackText, KeyLabel, KeyKind, KeyAmountV2)
    }

  /// One field as `serialize_to_map` builds it: entries are pushed into a
  /// `std::collections::HashMap`, so `entries` is an arbitrary build order.
  type FieldBuild = { variant: FieldVariant, entries: List[FieldEntry] }

  type FieldBytes =
      FSorted(Set[FieldEntry])
    | FInsertion(List[FieldEntry])

  /// `impl Serialize for SignablePayloadField` (lib.rs:354): the `HashMap` is
  /// collected into a `BTreeMap` and emitted from there, so the keys come out
  /// alphabetically whatever order the variant arm pushed them in.
  pure def serializeFieldToJson(f: FieldBuild): FieldBytes =
    FSorted(fieldEntrySet(f.entries))

  /// What emitting straight from the `HashMap` would have produced. Counterfactual
  /// only; never produced by an action.
  pure def hashMapFieldJson(f: FieldBuild): FieldBytes =
    FInsertion(f.entries)

  pure def fieldIsContentOrdered(b: FieldBytes): bool =
    match b {
      | FSorted(_)    => true
      | FInsertion(_) => false
    }

  pure def fieldKeys(entries: List[FieldEntry]): Set[FieldKey] =
    entries.foldl(Set(), (acc, e) => acc.union(Set(e.key)))

  /// The missing-field / unexpected-field guard in `Serialize` (lib.rs:367-389):
  /// serialization fails outright when the emitted key set is not exactly the
  /// expected one.
  pure def fieldShapeIsValid(f: FieldBuild): bool =
    fieldKeys(f.entries) == expectedKeys(f.variant)

  /// `verify_json_deterministic` (lib.rs:51): every object's keys must appear in
  /// alphabetical order. A content-ordered emission satisfies it by construction;
  /// an insertion-ordered one only when the builder happened to insert in order.
  pure def keysAreAlphabetical(b: FieldBytes): bool =
    match b {
      | FSorted(_)     => true
      | FInsertion(l)  => l.indices().forall(i =>
                            if (i == 0) true
                            else keyRank(l.nth(i - 1).key) < keyRank(l.nth(i).key))
    }

  // ===========================================================================
  // 4. The Solana intermediate output
  // ===========================================================================

  /// One `named_accounts` binding (role -> account key). Upstream this is a
  /// `HashMap`; `SolanaParsedInstructionDataIo` re-keys it into a `BTreeMap`
  /// (intermediate.rs:113, :235).
  type Binding = { name: str, addr: str }

  /// A revision of the `SolanaIntermediateOutput` borsh schema.
  /// `SOLANA_INTERMEDIATE_SCHEMA_VERSION` is the FIRST field (intermediate.rs:47-49),
  /// so any shape change that bumps it is visible in the very first bytes rather
  /// than silently compatible.
  type Shape = { version: int, hasExtraField: bool }

  pure val SHAPE_V1: Shape = { version: 1, hasExtraField: false }
  /// A hypothetical next revision: one more field, and the version bumped with it.
  pure val SHAPE_V2: Shape = { version: 2, hasExtraField: true }

  type IntermediateBuild = {
    shape: Shape,
    // upstream `HashMap`, in whatever order it iterated
    namedAccounts: List[Binding],
    callArgs: JsonBuild,
    // `account_keys` is a `Vec`: its order IS content, not build order
    accountKeys: List[str],
  }

  type IntermediateBytes = {
    shape: Shape,
    namedAccounts: Set[Binding],
    callArgsJson: JsonBytes,
    accountKeys: List[str],
  }

  type IntermediateContent = {
    shape: Shape,
    namedAccounts: Set[Binding],
    callArgs: JsonContent,
    accountKeys: List[str],
  }

  /// `SolanaIntermediateOutput::from` + `borsh::to_vec` (intermediate.rs:233-291,
  /// visualsign.rs:422).
  pure def borshIntermediate(i: IntermediateBuild): IntermediateBytes = {
    shape: i.shape,
    namedAccounts: bindingSet(i.namedAccounts),
    callArgsJson: canonicalArgsJson(i.callArgs),
    accountKeys: i.accountKeys,
  }

  pure def intermediateContent(i: IntermediateBuild): IntermediateContent = {
    shape: i.shape,
    namedAccounts: bindingSet(i.namedAccounts),
    callArgs: argsContent(i.callArgs),
    accountKeys: i.accountKeys,
  }

  /// The content of an intermediate output with its schema shape projected away --
  /// "the same logical decode, encoded under a different schema revision".
  pure def intermediatePayload(c: IntermediateContent): IntermediateContent =
    { ...c, shape: SHAPE_V1 }

  // ===========================================================================
  // 5. The signed bytes -- where all three mechanisms compose
  // ===========================================================================

  /// Whether the caller opted into the structured decode
  /// (`ParseRequest.include_intermediate_output`). An absent intermediate output is
  /// the empty `bytes` field, which is what `signing_digest_bytes` treats as
  /// "append nothing".
  type MaybeIntermediate = NoIntermediate | WithIntermediate(IntermediateBuild)

  type MaybeIntermediateContent =
      NoIntermediateContent
    | WithIntermediateContent(IntermediateContent)

  /// Everything one signing consumes, each part still in the order its builder
  /// produced it.
  type SigningBuild = {
    payloadField: FieldBuild,
    // the raw request bytes behind `input_payload_digest`
    inputPayload: str,
    metadata: List[MetaEntry],
    intermediate: MaybeIntermediate,
  }

  /// The four fields the DERIVED borsh encoding covers. `intermediate_output` is
  /// `#[borsh(skip)]` (codegen/src/main.rs:67-70), so it is absent here by
  /// construction.
  type DerivedBorsh = {
    parsedPayload: FieldBytes,
    inputPayloadDigest: str,
    metadataDigest: MapBytes,
    // the legacy `signable_payload` mirror (parse.rs:103)
    signablePayload: FieldBytes,
  }

  /// What was appended after the derived encoding: nothing, or the borsh `Vec<u8>`
  /// of the intermediate bytes.
  type Appended = ANothing | ABytes(IntermediateBytes)

  type SignedBytes = { derived: DerivedBorsh, appended: Appended }

  pure def derivedBorshOf(s: SigningBuild): DerivedBorsh = {
    parsedPayload: serializeFieldToJson(s.payloadField),
    inputPayloadDigest: s.inputPayload,
    metadataDigest: borshChainMetadata(s.metadata),
    signablePayload: serializeFieldToJson(s.payloadField),
  }

  /// `borsh::to_vec(payload)` alone -- the pre-feature four-field encoding an
  /// intermediate-unaware verifier reproduces.
  pure def legacyBorshBytes(s: SigningBuild): SignedBytes =
    { derived: derivedBorshOf(s), appended: ANothing }

  /// `signing_digest_bytes` (parse.rs:145): the derived encoding, plus the
  /// intermediate bytes appended only when the field is non-empty.
  pure def signingDigestBytes(s: SigningBuild): SignedBytes = {
    derived: derivedBorshOf(s),
    appended: match s.intermediate {
      | NoIntermediate     => ANothing
      | WithIntermediate(io) => ABytes(borshIntermediate(io))
    },
  }

  /// The logical content one signing binds: order-free wherever a map is involved,
  /// order-preserving wherever a vector is.
  type SigningContent = {
    payloadField: Set[FieldEntry],
    variant: FieldVariant,
    inputPayload: str,
    metadata: Set[MetaEntry],
    intermediate: MaybeIntermediateContent,
  }

  pure def signingContent(s: SigningBuild): SigningContent = {
    payloadField: fieldEntrySet(s.payloadField.entries),
    variant: s.payloadField.variant,
    inputPayload: s.inputPayload,
    metadata: metaSet(s.metadata),
    intermediate: match s.intermediate {
      | NoIntermediate      => NoIntermediateContent
      | WithIntermediate(io) => WithIntermediateContent(intermediateContent(io))
    },
  }

  // ===========================================================================
  // State -- the encoding ledger
  // ===========================================================================

  type ArgsRecord = { content: JsonContent, bytes: JsonBytes, build: JsonBuild }
  type MetaRecord = { content: Set[MetaEntry], bytes: MapBytes, build: List[MetaEntry] }
  type InterRecord = { content: IntermediateContent, bytes: IntermediateBytes,
                       build: IntermediateBuild }
  type FieldRecord = { content: Set[FieldEntry], variant: FieldVariant, bytes: FieldBytes,
                       build: List[FieldEntry] }
  type SignRecord = { content: SigningContent, bytes: SignedBytes, build: SigningBuild }

  /// Every serialization the system performed, oldest first, each one recording the
  /// logical content that went in beside the bytes that came out. The history is
  /// state because determinism is a property RELATING two encodings, not a property
  /// of any single one.
  type Ledger = {
    args: List[ArgsRecord],
    metadata: List[MetaRecord],
    intermediate: List[InterRecord],
    payloadFields: List[FieldRecord],
    signings: List[SignRecord],
    // field builds that `Serialize` refused because the key set was wrong
    rejectedFields: List[FieldBuild],
  }

  pure val EMPTY_LEDGER: Ledger = {
    args: List(),
    metadata: List(),
    intermediate: List(),
    payloadFields: List(),
    signings: List(),
    rejectedFields: List(),
  }

  var ledger: Ledger

  // ===========================================================================
  // The input domain -- every build order the test suite actually exercises
  // ===========================================================================

  // -- args objects (intermediate.rs's `canonical_args_json` tests) -------------

  pure val NESTED_FORWARD: List[Leaf] = List({ name: "nalpha", num: 2 }, { name: "nzeta", num: 1 })
  pure val NESTED_REVERSE: List[Leaf] = List({ name: "nzeta", num: 1 }, { name: "nalpha", num: 2 })
  pure val ARRAY_FORWARD: List[Leaf] = List({ name: "balpha", num: 4 }, { name: "bzeta", num: 3 })
  pure val ARRAY_REVERSE: List[Leaf] = List({ name: "bzeta", num: 3 }, { name: "balpha", num: 4 })

  /// `canonical_args_json_alphabetizes_keys`: two flat maps, opposite insertion
  /// orders, same entries.
  pure val ARGS_FLAT_FORWARD: JsonBuild =
    List({ name: "alpha", value: VNum(2) }, { name: "zeta", value: VNum(1) })
  pure val ARGS_FLAT_REVERSE: JsonBuild =
    List({ name: "zeta", value: VNum(1) }, { name: "alpha", value: VNum(2) })

  /// `canonical_args_json_alphabetizes_nested_keys`: the same content reordered at
  /// the top level, inside a nested object, AND inside an object that is an array
  /// element -- the case a top-level-only sort gets wrong.
  pure val ARGS_NESTED_A: JsonBuild = List(
    { name: "zeta", value: VObj(NESTED_REVERSE) },
    { name: "alpha", value: VArr(List(ARRAY_REVERSE)) },
  )
  pure val ARGS_NESTED_B: JsonBuild = List(
    { name: "alpha", value: VArr(List(ARRAY_FORWARD)) },
    { name: "zeta", value: VObj(NESTED_FORWARD) },
  )

  /// `parsed_instruction_data_io_round_trip`'s args, and an empty map.
  pure val ARGS_TRANSFER: JsonBuild =
    List({ name: "amount", value: VNum(42) }, { name: "recipient", value: VNum(7) })
  pure val ARGS_EMPTY: JsonBuild = List()

  pure val ARGS_BUILDS: Set[JsonBuild] = Set(
    ARGS_FLAT_FORWARD, ARGS_FLAT_REVERSE,
    ARGS_NESTED_A, ARGS_NESTED_B,
    ARGS_TRANSFER, ARGS_EMPTY,
  )

  // -- chain metadata (`metadata_digest_is_deterministic`) ---------------------

  pure val ABI_TRANSFER: MetaEntry = { addr: "0xaaaa", abi: "transfer" }
  pure val ABI_APPROVE: MetaEntry = { addr: "0xbbbb", abi: "approve" }

  pure val META_FORWARD: List[MetaEntry] = List(ABI_TRANSFER, ABI_APPROVE)
  pure val META_REVERSE: List[MetaEntry] = List(ABI_APPROVE, ABI_TRANSFER)
  pure val META_SINGLE: List[MetaEntry] = List(ABI_TRANSFER)
  pure val META_EMPTY: List[MetaEntry] = List()

  pure val METADATA_BUILDS: Set[List[MetaEntry]] =
    Set(META_FORWARD, META_REVERSE, META_SINGLE, META_EMPTY)

  // -- payload fields ----------------------------------------------------------

  /// An `AmountV2` field whose variant arm pushed its four keys into the `HashMap`
  /// in a non-alphabetical order.
  pure val AMOUNT_SHUFFLED: FieldBuild = { variant: VariantAmountV2, entries: List(
    { key: KeyKind, text: "amount_v2" },
    { key: KeyFallbackText, text: "1.5 ETH" },
    { key: KeyAmountV2, text: "{amount}" },
    { key: KeyLabel, text: "Amount" },
  ) }

  /// The same field, same entries, built in alphabetical order.
  pure val AMOUNT_ALPHABETICAL: FieldBuild = { variant: VariantAmountV2, entries: List(
    { key: KeyAmountV2, text: "{amount}" },
    { key: KeyFallbackText, text: "1.5 ETH" },
    { key: KeyLabel, text: "Amount" },
    { key: KeyKind, text: "amount_v2" },
  ) }

  pure val TEXT_FIELD: FieldBuild = { variant: VariantText, entries: List(
    { key: KeyLabel, text: "Memo" },
    { key: KeyText, text: "{memo}" },
    { key: KeyFallbackText, text: "hello" },
    { key: KeyKind, text: "text" },
  ) }

  /// A variant arm that forgot to emit `Label`: `Serialize` refuses rather than
  /// emitting a short object (lib.rs:367).
  pure val TEXT_MISSING_LABEL: FieldBuild = { variant: VariantText, entries: List(
    { key: KeyKind, text: "text" },
    { key: KeyFallbackText, text: "hello" },
    { key: KeyText, text: "{memo}" },
  ) }

  /// A variant arm that emitted a key belonging to another variant (lib.rs:380).
  pure val TEXT_UNEXPECTED_KEY: FieldBuild = { variant: VariantText, entries: List(
    { key: KeyKind, text: "text" },
    { key: KeyFallbackText, text: "hello" },
    { key: KeyLabel, text: "Memo" },
    { key: KeyAmountV2, text: "{amount}" },
  ) }

  pure val FIELD_BUILDS: Set[FieldBuild] =
    Set(AMOUNT_SHUFFLED, AMOUNT_ALPHABETICAL, TEXT_FIELD, TEXT_MISSING_LABEL, TEXT_UNEXPECTED_KEY)

  // -- intermediate outputs ----------------------------------------------------

  pure val ACC_MINT: Binding = { name: "mint", addr: "Mint11111111111111" }
  pure val ACC_AUTHORITY: Binding = { name: "authority", addr: "Auth1111111111111" }

  /// `parsed_instruction_data_io_round_trip`: the upstream `HashMap` can hand the
  /// two bindings over in either order.
  pure val IO_FORWARD: IntermediateBuild = {
    shape: SHAPE_V1,
    namedAccounts: List(ACC_MINT, ACC_AUTHORITY),
    callArgs: ARGS_TRANSFER,
    accountKeys: List("A1", "B2"),
  }
  pure val IO_REVERSE: IntermediateBuild = { ...IO_FORWARD, namedAccounts: List(ACC_AUTHORITY, ACC_MINT) }

  /// Same bindings and same args CONTENT, but the nested args were built the other
  /// way round -- determinism has to survive that too.
  pure val IO_NESTED_A: IntermediateBuild = { ...IO_FORWARD, callArgs: ARGS_NESTED_A }
  pure val IO_NESTED_B: IntermediateBuild = { ...IO_REVERSE, callArgs: ARGS_NESTED_B }

  /// The same logical decode encoded under a bumped schema shape.
  pure val IO_NEXT_SHAPE: IntermediateBuild = { ...IO_FORWARD, shape: SHAPE_V2 }

  /// `account_keys` is a `Vec`, so swapping two elements is a DIFFERENT decode, not
  /// a different build order.
  pure val IO_SWAPPED_KEYS: IntermediateBuild = { ...IO_FORWARD, accountKeys: List("B2", "A1") }

  pure val INTERMEDIATE_BUILDS: Set[IntermediateBuild] =
    Set(IO_FORWARD, IO_REVERSE, IO_NESTED_A, IO_NESTED_B, IO_NEXT_SHAPE, IO_SWAPPED_KEYS)

  // -- signings ----------------------------------------------------------------

  /// Two requests carrying the SAME logical transaction, with every map built the
  /// opposite way round: the payload field map, the ABI map. This pair is the
  /// component's headline scenario.
  pure val SIGN_ORDER_A: SigningBuild = {
    payloadField: AMOUNT_SHUFFLED,
    inputPayload: "ethtx",
    metadata: META_FORWARD,
    intermediate: NoIntermediate,
  }
  pure val SIGN_ORDER_B: SigningBuild = {
    payloadField: AMOUNT_ALPHABETICAL,
    inputPayload: "ethtx",
    metadata: META_REVERSE,
    intermediate: NoIntermediate,
  }

  /// `signing_digest_appends_intermediate_when_present`.
  pure val SIGN_WITH_INTERMEDIATE: SigningBuild = {
    payloadField: TEXT_FIELD,
    inputPayload: "solanatx",
    metadata: META_EMPTY,
    intermediate: WithIntermediate(IO_FORWARD),
  }
  /// The same request with the intermediate output's maps built the other way.
  pure val SIGN_WITH_INTERMEDIATE_REORDERED: SigningBuild =
    { ...SIGN_WITH_INTERMEDIATE, intermediate: WithIntermediate(IO_REVERSE) }

  /// The same request WITHOUT the opt-in structured decode: its bytes must be the
  /// legacy four-field encoding.
  pure val SIGN_WITHOUT_INTERMEDIATE: SigningBuild =
    { ...SIGN_WITH_INTERMEDIATE, intermediate: NoIntermediate }

  /// A genuinely different transaction -- different bytes are REQUIRED here.
  pure val SIGN_OTHER_TRANSACTION: SigningBuild =
    { ...SIGN_ORDER_A, inputPayload: "othertx" }

  pure val SIGNING_BUILDS: Set[SigningBuild] = Set(
    SIGN_ORDER_A, SIGN_ORDER_B,
    SIGN_WITH_INTERMEDIATE, SIGN_WITH_INTERMEDIATE_REORDERED, SIGN_WITHOUT_INTERMEDIATE,
    SIGN_OTHER_TRANSACTION,
  )

  // ===========================================================================
  // Actions -- one real serialization each
  // ===========================================================================

  action init: bool = {
    ledger' = EMPTY_LEDGER
  }

  /// `canonical_args_json(&value.program_call_args)`
  /// (visualsign-solana/src/intermediate.rs:189).
  action canonicalize_call_args(build: JsonBuild): bool = all {
    ledger' = { ...ledger, args: ledger.args.append({
      content: argsContent(build),
      bytes: canonicalArgsJson(build),
      build: build,
    }) },
  }

  /// `borsh::to_vec(&metadata)` on the `metadata_digest` path
  /// (parser/app/src/routes/parse.rs:93).
  action encode_chain_metadata(build: List[MetaEntry]): bool = all {
    ledger' = { ...ledger, metadata: ledger.metadata.append({
      content: metaSet(build),
      bytes: borshChainMetadata(build),
      build: build,
    }) },
  }

  /// `SolanaIntermediateOutput::from(..)` followed by `borsh::to_vec`
  /// (visualsign-solana/src/core/visualsign.rs:422, intermediate.rs:270).
  action encode_intermediate_output(build: IntermediateBuild): bool = all {
    ledger' = { ...ledger, intermediate: ledger.intermediate.append({
      content: intermediateContent(build),
      bytes: borshIntermediate(build),
      build: build,
    }) },
  }

  /// `impl Serialize for SignablePayloadField` (visualsign/src/lib.rs:354): the
  /// key set checked out, so the `BTreeMap` is emitted.
  action serialize_payload_field(build: FieldBuild): bool = all {
    fieldShapeIsValid(build),
    ledger' = { ...ledger, payloadFields: ledger.payloadFields.append({
      content: fieldEntrySet(build.entries),
      variant: build.variant,
      bytes: serializeFieldToJson(build),
      build: build.entries,
    }) },
  }

  /// The same `Serialize` impl refusing: a key is missing or does not belong to
  /// this variant, so no bytes are emitted at all (lib.rs:367-389).
  action reject_payload_field_shape_mismatch(build: FieldBuild): bool = all {
    not(fieldShapeIsValid(build)),
    ledger' = { ...ledger, rejectedFields: ledger.rejectedFields.append(build) },
  }

  /// `signing_digest_bytes` (parser/app/src/routes/parse.rs:145): serialize the
  /// payload, borsh the metadata, borsh the four derived fields, and append the
  /// intermediate output only when it is non-empty.
  action compute_signing_digest(build: SigningBuild): bool = all {
    fieldShapeIsValid(build.payloadField),
    ledger' = { ...ledger, signings: ledger.signings.append({
      content: signingContent(build),
      bytes: signingDigestBytes(build),
      build: build,
    }) },
  }

  action step: bool = {
    nondet argsBuild = ARGS_BUILDS.oneOf()
    nondet metaBuild = METADATA_BUILDS.oneOf()
    nondet fieldBuild = FIELD_BUILDS.oneOf()
    nondet interBuild = INTERMEDIATE_BUILDS.oneOf()
    nondet signBuild = SIGNING_BUILDS.oneOf()
    any {
      canonicalize_call_args(argsBuild),
      encode_chain_metadata(metaBuild),
      encode_intermediate_output(interBuild),
      serialize_payload_field(fieldBuild),
      reject_payload_field_shape_mismatch(fieldBuild),
      compute_signing_digest(signBuild),
    }
  }

  // ===========================================================================
  // Observations -- the behaviors worth measuring test coverage for
  // ===========================================================================

  /// Two flat args maps with the same entries were canonicalized from OPPOSITE
  /// insertion orders and produced identical JSON.
  val flat_args_order_collapsed: bool =
    ledger.args.indices().exists(i => ledger.args.indices().exists(j => {
      val a = ledger.args.nth(i)
      val b = ledger.args.nth(j)
      and {
        a.build != b.build,
        a.content == b.content,
        a.bytes == b.bytes,
      }
    }))

  /// The nested case: two args objects whose NESTED objects (inside a value and
  /// inside an array element) were built in opposite orders still canonicalized to
  /// the same JSON -- and a top-level-only sort would have produced different
  /// bytes. This is the observation that proves the recursive walk is load-bearing.
  val nested_args_order_collapsed: bool =
    ledger.args.indices().exists(i => ledger.args.indices().exists(j => {
      val a = ledger.args.nth(i)
      val b = ledger.args.nth(j)
      and {
        a.content == b.content,
        a.bytes == b.bytes,
        topLevelSortedOnly(a.build) != topLevelSortedOnly(b.build),
      }
    }))

  /// Two `chain_metadata` values with the same ABI entries were borsh-encoded from
  /// opposite insertion orders and hashed to the same `metadata_digest`.
  val metadata_order_collapsed: bool =
    ledger.metadata.indices().exists(i => ledger.metadata.indices().exists(j => {
      val a = ledger.metadata.nth(i)
      val b = ledger.metadata.nth(j)
      and {
        a.build != b.build,
        a.content == b.content,
        a.bytes == b.bytes,
      }
    }))

  /// A `SignablePayloadField` whose variant arm pushed keys in a NON-alphabetical
  /// order was still emitted with alphabetical keys.
  val payload_field_keys_realphabetized: bool =
    ledger.payloadFields.indices().exists(i => {
      val r = ledger.payloadFields.nth(i)
      and {
        not(keysAreAlphabetical(hashMapFieldJson({ variant: r.variant, entries: r.build }))),
        keysAreAlphabetical(r.bytes),
      }
    })

  /// A field build with the wrong key set was refused instead of being emitted.
  val payload_field_shape_mismatch_rejected: bool =
    ledger.rejectedFields.length() > 0

  /// Two intermediate outputs whose `named_accounts` came out of the upstream
  /// `HashMap` in opposite orders encoded to identical borsh bytes.
  val named_accounts_order_collapsed: bool =
    ledger.intermediate.indices().exists(i => ledger.intermediate.indices().exists(j => {
      val a = ledger.intermediate.nth(i)
      val b = ledger.intermediate.nth(j)
      and {
        a.build.namedAccounts != b.build.namedAccounts,
        a.content == b.content,
        a.bytes == b.bytes,
      }
    }))

  /// The same logical decode was encoded under two different schema shapes and the
  /// bytes differed -- a shape change is observable, never silently compatible.
  val schema_shape_change_observed: bool =
    ledger.intermediate.indices().exists(i => ledger.intermediate.indices().exists(j => {
      val a = ledger.intermediate.nth(i)
      val b = ledger.intermediate.nth(j)
      and {
        a.content.shape != b.content.shape,
        intermediatePayload(a.content) == intermediatePayload(b.content),
        a.bytes != b.bytes,
      }
    }))

  /// Two structurally identical requests, every map built the opposite way round,
  /// produced byte-identical signed bytes. THE headline behavior.
  val identical_content_signed_identically: bool =
    ledger.signings.indices().exists(i => ledger.signings.indices().exists(j => {
      val a = ledger.signings.nth(i)
      val b = ledger.signings.nth(j)
      and {
        a.build != b.build,
        a.content == b.content,
        a.bytes == b.bytes,
      }
    }))

  /// Two genuinely different transactions were signed and their bytes differed --
  /// the encoding does not collapse distinct content.
  val distinct_content_signed_differently: bool =
    ledger.signings.indices().exists(i => ledger.signings.indices().exists(j => {
      val a = ledger.signings.nth(i)
      val b = ledger.signings.nth(j)
      and {
        a.content != b.content,
        a.bytes != b.bytes,
      }
    }))

  /// Something was signed with no `intermediate_output`, so the signed bytes are
  /// exactly the legacy four-field borsh encoding.
  val signed_with_legacy_encoding: bool =
    ledger.signings.foldl(false, (seen, r) =>
      seen or (r.bytes.appended == ANothing and r.bytes == legacyBorshBytes(r.build)))

  /// Something was signed WITH an `intermediate_output`, so the appended bytes
  /// pushed the digest away from the legacy encoding.
  val signed_with_appended_intermediate: bool =
    ledger.signings.foldl(false, (seen, r) =>
      seen or (r.bytes.appended != ANothing and r.bytes != legacyBorshBytes(r.build)))

  /// The same request was signed both with and without the opt-in structured
  /// decode, and only the derived four-field prefix stayed the same -- the exact
  /// backward-compatibility scenario.
  val intermediate_opt_in_kept_prefix_stable: bool =
    ledger.signings.indices().exists(i => ledger.signings.indices().exists(j => {
      val a = ledger.signings.nth(i)
      val b = ledger.signings.nth(j)
      and {
        a.bytes.appended == ANothing,
        b.bytes.appended != ANothing,
        a.bytes.derived == b.bytes.derived,
        a.bytes != b.bytes,
      }
    }))

  // ===========================================================================
  // Invariants
  // ===========================================================================

  /// THE headline safety property: two structurally identical inputs always produce
  /// identical signed bytes, whatever order their maps were built in.
  val same_content_always_signs_the_same_bytes: bool =
    ledger.signings.indices().forall(i => ledger.signings.indices().forall(j => {
      val a = ledger.signings.nth(i)
      val b = ledger.signings.nth(j)
      a.content == b.content implies a.bytes == b.bytes
    }))

  /// The converse, and what makes the guarantee useful: distinct logical content is
  /// always observable as distinct bytes rather than silently colliding.
  val distinct_content_never_shares_signed_bytes: bool =
    ledger.signings.indices().forall(i => ledger.signings.indices().forall(j => {
      val a = ledger.signings.nth(i)
      val b = ledger.signings.nth(j)
      a.bytes == b.bytes implies a.content == b.content
    }))

  /// `canonical_args_json` depends only on the args CONTENT -- at every nesting
  /// level, not just the top one.
  val canonical_args_depend_only_on_content: bool =
    ledger.args.indices().forall(i => ledger.args.indices().forall(j => {
      val a = ledger.args.nth(i)
      val b = ledger.args.nth(j)
      a.content == b.content iff a.bytes == b.bytes
    }))

  /// `metadata_digest` depends only on the ABI map's content: the proto map is a
  /// `BTreeMap` by codegen, so insertion order cannot reach the hash.
  val metadata_digest_depends_only_on_content: bool =
    ledger.metadata.indices().forall(i => ledger.metadata.indices().forall(j => {
      val a = ledger.metadata.nth(i)
      val b = ledger.metadata.nth(j)
      a.content == b.content iff a.bytes == b.bytes
    }))

  /// The Solana intermediate output's borsh bytes depend only on its content.
  val intermediate_bytes_depend_only_on_content: bool =
    ledger.intermediate.indices().forall(i => ledger.intermediate.indices().forall(j => {
      val a = ledger.intermediate.nth(i)
      val b = ledger.intermediate.nth(j)
      a.content == b.content iff a.bytes == b.bytes
    }))

  /// A serialized `SignablePayloadField` depends only on its key/value content, and
  /// never on the order the variant arm pushed entries into the `HashMap`.
  val payload_field_json_depends_only_on_content: bool =
    ledger.payloadFields.indices().forall(i => ledger.payloadFields.indices().forall(j => {
      val a = ledger.payloadFields.nth(i)
      val b = ledger.payloadFields.nth(j)
      a.content == b.content iff a.bytes == b.bytes
    }))

  /// Every map that reaches a hash is emitted in content-determined order. No
  /// insertion-ordered map is ever produced on a signing path.
  val every_hashed_map_is_content_ordered: bool =
    and {
      ledger.metadata.foldl(true, (ok, r) => ok and mapIsContentOrdered(r.bytes)),
      ledger.payloadFields.foldl(true, (ok, r) => ok and fieldIsContentOrdered(r.bytes)),
      ledger.args.foldl(true, (ok, r) => ok and jsonIsContentOrdered(r.bytes)),
      ledger.intermediate.foldl(true, (ok, r) => ok and jsonIsContentOrdered(r.bytes.callArgsJson)),
      ledger.signings.foldl(true, (ok, r) => and {
        ok,
        mapIsContentOrdered(r.bytes.derived.metadataDigest),
        fieldIsContentOrdered(r.bytes.derived.parsedPayload),
      }),
    }

  /// `verify_deterministic_ordering` would pass on every field JSON the system
  /// emitted (visualsign/src/lib.rs:23).
  val emitted_field_json_is_always_alphabetical: bool =
    ledger.payloadFields.foldl(true, (ok, r) => ok and keysAreAlphabetical(r.bytes))

  /// Nothing is ever emitted for a field whose key set does not match the variant's
  /// expected fields.
  val only_well_shaped_fields_are_emitted: bool =
    ledger.payloadFields.foldl(true, (ok, r) =>
      ok and r.content.map(e => e.key) == expectedKeys(r.variant))

  /// Backward compatibility: with an empty `intermediate_output` the signed bytes
  /// are byte-for-byte the legacy four-field borsh encoding, so a verifier that
  /// predates the field keeps verifying.
  val empty_intermediate_matches_legacy_encoding: bool =
    ledger.signings.foldl(true, (ok, r) =>
      ok and (r.build.intermediate == NoIntermediate implies r.bytes == legacyBorshBytes(r.build)))

  /// A non-empty `intermediate_output` always changes the signed bytes, so an
  /// intermediate-unaware consumer rejects rather than mis-verifies.
  val non_empty_intermediate_changes_signed_bytes: bool =
    ledger.signings.foldl(true, (ok, r) =>
      ok and (r.build.intermediate != NoIntermediate implies r.bytes != legacyBorshBytes(r.build)))

  /// `#[borsh(skip)]` really does keep `intermediate_output` out of the derived
  /// encoding: the four-field prefix is the same whether or not the field is set.
  val derived_prefix_ignores_intermediate_output: bool =
    ledger.signings.foldl(true, (ok, r) =>
      ok and r.bytes.derived == legacyBorshBytes(r.build).derived)

  /// A bump of the intermediate output's schema shape always changes the bytes,
  /// even when the decoded content underneath is identical.
  val schema_shape_change_always_changes_bytes: bool =
    ledger.intermediate.indices().forall(i => ledger.intermediate.indices().forall(j => {
      val a = ledger.intermediate.nth(i)
      val b = ledger.intermediate.nth(j)
      (intermediatePayload(a.content) == intermediatePayload(b.content)
        and a.content.shape != b.content.shape) implies a.bytes != b.bytes
    }))

  /// Vectors are NOT order-freed. Two decodes carrying the same `account_keys`
  /// elements in a DIFFERENT order are different decodes, and the encoding keeps
  /// them apart -- the `BTreeMap`/canonicalization treatment is applied to maps
  /// only, never to the `Vec` fields whose order is itself content.
  val vector_order_is_part_of_content: bool =
    ledger.intermediate.indices().forall(i => ledger.intermediate.indices().forall(j => {
      val a = ledger.intermediate.nth(i)
      val b = ledger.intermediate.nth(j)
      (and {
        strSet(a.content.accountKeys) == strSet(b.content.accountKeys),
        a.content.accountKeys != b.content.accountKeys,
      }) implies a.bytes != b.bytes
    }))
}
