// Metadata Signature Trust -- admission control for caller-supplied ABI/IDL metadata.
//
// Models the trust boundary that decides which caller-supplied decoders may shape what a
// human sees in the signing payload:
//
//   env signer list -> SignerAllowlist          (visualsign/src/signing.rs:80)
//     Ethereum: authorized_abi_signers()        (abi_metadata.rs:351, fresh per call)
//     Solana:   authorized_idl_signers()        (idl/signature.rs:~215, OnceLock-cached)
//
//   caller metadata map -> per-entry admission loop
//     Ethereum: try_extract_from_chain_metadata (abi_metadata.rs:76)
//         parse address -> size bound -> validate_abi_signature (abi_metadata.rs:268)
//         -> resolve_abi_kind (abi_metadata.rs:~165) -> register_embedded_abi
//         -> AbiRegistry::map_address_with_type  (abi_registry.rs:~135)
//     Solana:   extract_idl_mappings_with_signers (core/visualsign.rs:196)
//         parse pubkey -> trusted-program filter -> size bound
//         -> validate_idl_signature (idl/signature.rs:~150) -> reserved-name guard
//         -> IdlRegistry::from_idl_mappings     (idl/mod.rs:66)
//
//   shared prehash: SHA-256 over le_u64-length-prefixed
//                   DOMAIN || chain_tag || scope || body   (signing.rs:139)
//
// WHAT THIS COVERS
//   * the full reject taxonomy of both admission loops, one outcome class per `continue`
//     site, including every distinct signature-validation failure cause in the order the
//     verifier checks them
//   * the allowlist as a mechanism: how it is built from the env list, what a malformed
//     env entry does, and that an empty allowlist rejects every SIGNED entry while every
//     UNSIGNED entry is still admitted
//   * the prehash as a length-prefixed encoding, so its injectivity over
//     (domain, chain_tag, scope, body) is a checkable property rather than an assumption
//   * what a signature does and does NOT bind: `abi_type` and `implementation_address`
//     are outside the signed preimage, as are `issuer` and `timestamp`
//   * the registry state both loops write, and the guarded reads that consume it
//     (get_abi_for_address, get_implementation_abi with all four unresolved causes,
//     has_idl / get_idl / get_program_name / get_idl_name)
//   * the direct `AbiRegistry` writers (`register_abi`, `map_address`,
//     `map_address_with_type`) -- `map_address_with_type` never checks that the ABI name
//     it maps to was registered
//
// WHAT THIS DOES NOT COVER
//   * real cryptography. A hash is modelled by its own preimage (injective by
//     construction of the encoder, which is exactly the property being checked), and a
//     signature by the (prehash, signer) pair it was minted over. ECDSA/ed25519
//     malleability, small-order keys and DER framing collapse into the
//     "malformed material" reject class, which is where the real code puts them too.
//   * SEC1 compressed/uncompressed canonicalisation. `canonical_pubkey_from_hex` maps
//     both encodings of one key onto identical bytes, so the model uses one key id.
//   * the calldata decoder itself. A "decoded rendering" is identified by the ABI body
//     that drove it -- which is all the proxy-routing properties need.
//   * byte-exact sizes. `MAX_ABI_JSON_BYTES` / `MAX_IDL_JSON_BYTES` are modelled as a
//     per-body "oversized" fact, since the code's only decision is over/under.
//   * concurrency. Both loops are single-threaded over a BTreeMap; the model steps one
//     entry at a time in whatever order the simulator picks, which is a superset of
//     BTreeMap key order.
//
// BUGS THIS MODEL CAN CATCH
//   1. A prehash whose fields are concatenated without their le_u64 length prefixes: two
//      distinct (scope, body) splits collide and one curator signature validates for a
//      second contract or program (`prehash_encoding_is_injective`).
//   2. A mapping installed under a name that was never registered -- `map_address_with_type`
//      checks nothing, so a mapped address decodes to nothing
//      (`every_mapping_resolves_to_a_registered_abi`).
//   3. A second caller map key spelling one address replacing the mapping an earlier
//      admitted entry installed, while the displaced body still reports as registered
//      (`admitted_metadata_mapping_is_never_shadowed`).
//   4. A body admitted into the IDL registry that no step ever parsed, so `has_idl` and
//      `get_idl` disagree (`advertised_idl_is_always_decodable`).
//   5. A signature that survives a change to the routing fields it never covered
//      (`curator_vouched_address_decodes_the_vouched_body`).
//
// REPLAY SURFACE
//   Every action argument is something the code can observe where the transition is
//   reported: the map key / program id string, the resolved chain id, the body id, the
//   proto abi_type, the implementation_address string, the signer key id. WHICH outcome
//   an entry got is carried by which action fires, never by an argument -- the loop does
//   not know an entry's fate before it runs the checks.
module metadata_signature_trust {

  import basicSpells.* from "./spells/basicSpells"

  // ===========================================================================
  // Bytes and the v1 prehash (visualsign/src/signing.rs)
  // ===========================================================================

  /// Byte strings are short int lists. A digest is modelled as its own preimage: the
  /// question this spec asks about the prehash is whether the ENCODING is injective, so
  /// abstracting SHA-256 as injective keeps exactly that question and drops nothing else.
  type Bytes = List[int]

  /// `METADATA_SIGNING_DOMAIN_V1` (signing.rs:~124), b"visualsign-metadata-v1".
  pure val DOMAIN_V1: Bytes = List(90, 91)
  /// `CHAIN_TAG_ETHEREUM`.
  pure val TAG_ETHEREUM: Bytes = List(1)
  /// `CHAIN_TAG_SOLANA`.
  pure val TAG_SOLANA: Bytes = List(2)

  /// `le_u64(field.len()) || field` -- the length prefix that makes the concatenation
  /// injective for arbitrary field contents.
  pure def lenPrefixed(raw: Bytes): Bytes = List(raw.length()).concat(raw)

  /// `metadata_signing_prehash_v1` (signing.rs:139).
  pure def prehashV1(tag: Bytes, scope: Bytes, body: Bytes): Bytes =
    lenPrefixed(DOMAIN_V1)
      .concat(lenPrefixed(tag))
      .concat(lenPrefixed(scope))
      .concat(lenPrefixed(body))

  /// The four fields the v1 preimage commits to. Nothing outside this set is bound by a
  /// signature, however official-looking it is.
  pure val PREHASH_COVERED_FIELDS: Set[str] = Set("domain", "chain_tag", "scope", "body")

  /// The `SignatureMetadata` fields any validation step actually reads
  /// (`validate_abi_signature` steps 1-7 / `validate_idl_signature`).
  pure val SIGNATURE_FIELDS_READ_BY_VALIDATION: Set[str] =
    Set("value", "algorithm", "public_key")

  // ===========================================================================
  // Identities (small, deliberately alias-rich, domains)
  // ===========================================================================

  /// Resolved chain ids the ABI prehash scope commits to.
  pure val ETH_CHAIN_IDS: Set[int] = Set(1, 137)

  /// The addresses the suite actually supplies, so a logged map key is a value this
  /// spec already knows. `ADDR_TETHER_LOWER` is the same address in a different
  /// spelling -- the alias surface; `ADDR_MALFORMED` is what the code fails to parse.
  pure val ADDR_TETHER: str = "0xdAC17F958D2ee523a2206206994597C13D831ec7"
  pure val ADDR_TETHER_LOWER: str = "0xdac17f958d2ee523a2206206994597c13d831ec7"
  pure val ADDR_PROXY: str = "0x1111111111111111111111111111111111111111"
  pure val ADDR_IMPL: str = "0x2222222222222222222222222222222222222222"
  pure val ADDR_UNSIGNED: str = "0x3333333333333333333333333333333333333333"
  pure val ADDR_MALFORMED: str = "not_an_address"

  /// Raw `BTreeMap<String, Abi>` keys as supplied by the caller.
  pure val ETH_MAP_KEYS: Set[str] = Set(
    ADDR_TETHER, ADDR_TETHER_LOWER, ADDR_PROXY, ADDR_IMPL, ADDR_UNSIGNED, ADDR_MALFORMED,
  )

  /// Map keys that do parse -- the domain a signature can legitimately be bound to.
  pure val ETH_PARSEABLE_KEYS: Set[str] = Set(ADDR_TETHER, ADDR_PROXY, ADDR_IMPL)

  /// Parsed `alloy_primitives::Address` values, in the checksummed spelling
  /// `Address::to_string()` produces.
  pure val ETH_ADDRS: Set[str] = Set(ADDR_TETHER, ADDR_PROXY, ADDR_IMPL, ADDR_UNSIGNED)

  /// `address.parse::<Address>()` (abi_metadata.rs:~92). Case-insensitive, so the two
  /// spellings of one address collapse onto a single parsed value.
  pure def parseEthAddr(mapKey: str): Option[str] =
    if (mapKey == ADDR_TETHER or mapKey == ADDR_TETHER_LOWER) Some(ADDR_TETHER)
    else if (mapKey == ADDR_PROXY) Some(ADDR_PROXY)
    else if (mapKey == ADDR_IMPL) Some(ADDR_IMPL)
    else if (mapKey == ADDR_UNSIGNED) Some(ADDR_UNSIGNED)
    else None

  pure def ethAddrOr(mapKey: str, fallback: str): str =
    match parseEthAddr(mapKey) { | Some(a) => a | None => fallback }

  pure def ethAddrBytes(addr: str): Bytes =
    if (addr == ADDR_TETHER) List(11)
    else if (addr == ADDR_PROXY) List(12)
    else if (addr == ADDR_IMPL) List(13)
    else if (addr == ADDR_UNSIGNED) List(14)
    else List(0)

  /// `ethereum_metadata_prehash` scope = 8-byte BE chain_id ++ 20-byte address.
  pure def ethScope(chainId: int, addr: str): Bytes = List(chainId).concat(ethAddrBytes(addr))

  pure def ethPrehash(chainId: int, addr: str, body: Bytes): Bytes =
    prehashV1(TAG_ETHEREUM, ethScope(chainId, addr), body)

  /// Caller-supplied ABI JSON bodies, by id. `abiBadJson` is well-formed enough to be
  /// carried and signed but fails `serde_json::from_str::<JsonAbi>` in `register_abi`;
  /// `abiOversize` exceeds `MAX_ABI_JSON_BYTES`.
  pure val ETH_BODIES: Set[str] = Set("abiA", "abiB", "abiBadJson", "abiOversize")

  pure def abiParseable(body: str): bool = body != "abiBadJson"
  pure def abiOversized(body: str): bool = body == "abiOversize"

  pure def abiBodyBytes(body: str): Bytes =
    if (body == "abiA") List(21)
    else if (body == "abiB") List(22)
    else if (body == "abiBadJson") List(23)
    else List(24)

  /// Proto `Abi.abi_type`. "unset" is `None`; "unknown_code" is a wire value outside the
  /// enum, which `AbiType::try_from` rejects -- both collapse to Unspecified.
  pure val ABI_TYPES: Set[str] = Set("unset", "unknown_code", "implementation", "proxy")

  /// The program ids the suite actually supplies. `PROG_SYSTEM` is a native program and
  /// `PROG_JUPITER` a built-in-IDL one (both canonically named, both trusted);
  /// `PROG_PRESET` stands for a preset-registered id -- trusted with NO canonical name,
  /// a case no current test reaches; `PROG_MALFORMED` fails `Pubkey::from_str`.
  pure val PROG_SYSTEM: str = "11111111111111111111111111111111"
  pure val PROG_SPL_TOKEN: str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
  pure val PROG_JUPITER: str = "JUP4Fb2cqiRUcaTHdrPC8h2gNsA2ETXiPDD33WcGuJB"
  pure val PROG_JUPITER_V6: str = "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4"
  pure val PROG_CUSTOM_A: str = "9xQeWvG816bUx9EPjHmaT23yvVM2ZWbrrpZb9PusVFin"
  pure val PROG_CUSTOM_B: str = "8yPdVuF705aTw8DOigLzS12xuUL1YVaqqoYa8OtrUEHm"
  /// A preset-registered program with no canonical name. The test that exercises this case
  /// discovers the id dynamically from `available_visualizers()`, so adding a preset ahead
  /// of metadao_futarchy would change the value the suite logs.
  pure val PROG_PRESET: str = "FUTARELBfJfQ8RDGhg1wdhddq1odMAJUePHFuBYfUxKq"
  pure val PROG_MALFORMED: str = "not_a_base58_pubkey"

  pure val SOL_PROGRAM_KEYS: Set[str] = Set(
    PROG_SYSTEM, PROG_SPL_TOKEN, PROG_JUPITER, PROG_JUPITER_V6, PROG_PRESET,
    PROG_CUSTOM_A, PROG_CUSTOM_B, PROG_MALFORMED,
  )

  /// Non-trusted, parseable program ids -- the only ones a caller IDL can be stored for.
  pure val SOL_OVERRIDABLE_PROGRAMS: Set[str] = Set(PROG_CUSTOM_A, PROG_CUSTOM_B)

  pure def solParsePubkey(programId: str): bool = programId != PROG_MALFORMED

  /// `canonical_name` (builtin_programs.rs:129).
  pure def solCanonicalName(programId: str): Option[str] =
    if (programId == PROG_SYSTEM) Some("System Program")
    else if (programId == PROG_SPL_TOKEN) Some("SPL Token Program")
    else if (programId == PROG_JUPITER) Some("Jupiter Swap")
    else if (programId == PROG_JUPITER_V6) Some("Jupiter Aggregator V6")
    else None

  /// `is_trusted_program` (builtin_programs.rs:209) -- canonical names PLUS every
  /// preset-registered program id.
  pure def solIsTrusted(programId: str): bool =
    solCanonicalName(programId) != None or programId == PROG_PRESET

  /// `ProgramType::from_program_id(..).is_some()` -- the built-in IDL set `has_idl`
  /// falls back to.
  pure def solHasBuiltinIdl(programId: str): bool =
    programId == PROG_JUPITER or programId == PROG_JUPITER_V6

  /// `is_reserved_canonical_name` (builtin_programs.rs:221) -- exact, case-sensitive.
  pure val RESERVED_CANONICAL_NAMES: Set[str] =
    Set("System Program", "SPL Token Program", "Jupiter Swap", "Jupiter Aggregator V6")

  pure def solProgBytes(programId: str): Bytes =
    if (programId == PROG_SYSTEM) List(41)
    else if (programId == PROG_SPL_TOKEN) List(42)
    else if (programId == PROG_JUPITER) List(43)
    else if (programId == PROG_JUPITER_V6) List(44)
    else if (programId == PROG_PRESET) List(45)
    else if (programId == PROG_CUSTOM_A) List(46)
    else if (programId == PROG_CUSTOM_B) List(47)
    else List(48)

  pure def solPrehash(programId: str, body: Bytes): Bytes =
    prehashV1(TAG_SOLANA, solProgBytes(programId), body)

  /// Caller-supplied IDL JSON bodies. `idlSpoofName` carries `metadata.name` =
  /// "System Program"; `idlBadJson` does not parse at all.
  pure val SOL_BODIES: Set[str] = Set("idlA", "idlSpoofName", "idlBadJson", "idlOversize")

  pure def idlParseable(body: str): bool = body != "idlBadJson"
  pure def idlOversized(body: str): bool = body == "idlOversize"

  /// `metadata.name` as read by BOTH `extract_name_from_idl_json` (core/visualsign.rs)
  /// and, independently, `IdlRegistry::from_idl_mappings` (idl/mod.rs:~78).
  pure def idlMetadataName(body: str): Option[str] =
    if (body == "idlA") Some("Alpha")
    else if (body == "idlSpoofName") Some("System Program")
    else if (body == "idlOversize") Some("Bulky")
    else None

  pure def idlBodyBytes(body: str): Bytes =
    if (body == "idlA") List(31)
    else if (body == "idlSpoofName") List(32)
    else if (body == "idlBadJson") List(33)
    else List(34)

  /// Proto `Idl.program_name` values a caller may supply. "My Wallet" is benign and walks
  /// past the impersonation guard; "System Program" is caught by it.
  pure val SOL_PROGRAM_NAMES: Set[str] = Set("My Wallet", "System Program")

  /// `format!("Program {}", &program_id[..8])` -- the fallback label. It can never
  /// collide with a reserved canonical name, so it is one opaque value here.
  pure val SOL_FALLBACK_NAME: str = "Program <id>"

  // ===========================================================================
  // Signer allowlist (SignerAllowlist + authorized_*_signers)
  // ===========================================================================

  /// Curator key ids. `devKey` is the `CLI_DEV_SIGNING_KEY_SEED` entry that only a
  /// `dev-signing` (or test) build of the Ethereum path adds.
  pure val SIGNER_KEYS: Set[str] = Set("curator", "rogue")
  pure val DEV_KEY: str = "devKey"

  /// Env entries for `VISUALSIGN_ETH_ABI_SIGNERS` / `VISUALSIGN_SOL_IDL_SIGNERS`.
  /// "MALFORMED" is an entry `canonical_pubkey_from_hex` returns `None` for: it is
  /// logged and skipped, and nothing else happens.
  pure val ENV_LISTS: Set[List[str]] = Set(
    List("curator"),
    List("MALFORMED"),
    List("curator", "MALFORMED"),
  )

  pure def envKeyIsValid(entry: str): bool = entry != "MALFORMED"

  pure def envKeys(env: List[str]): Set[str] =
    env.select(envKeyIsValid).foldl(Set(), (acc, k) => acc.union(Set(k)))

  pure def envDropped(env: List[str]): int =
    env.select(e => not(envKeyIsValid(e))).length()

  /// A built allowlist plus the configuration lineage needed to ask whether it still
  /// reflects what the deployment configured. `builtFromEnv` is the env snapshot taken
  /// when the allowlist was constructed.
  type SignerConfig = {
    env: List[str],
    keys: Set[str],
    dropped: int,
    built: bool,
    builtFromEnv: List[str],
  }

  pure val UNBUILT_SIGNERS: SignerConfig =
    { env: List(), keys: Set(), dropped: 0, built: false, builtFromEnv: List() }

  // ===========================================================================
  // Signature metadata and the verifier
  // ===========================================================================

  /// The `algorithm` metadata entry -- absent, or whatever string the caller put there.
  type AlgField = AlgMissing | AlgIs(str)

  /// The `public_key` metadata entry. `KeyMalformed` covers bad hex, a bad SEC1 point and
  /// a bad ed25519 point alike: the code turns all three into one reject.
  type KeyField = KeyMissing | KeyMalformed | KeyIs(str)

  /// The `value` field. `SigMalformed` covers bad hex, bad DER and wrong ed25519 length.
  /// `SigOver` records exactly what a real signature proves: some key signed some prehash.
  type SigValue = SigMalformed | SigOver({ prehash: Bytes, signer: str })

  type SigMeta = {
    value: SigValue,
    algorithm: AlgField,
    publicKey: KeyField,
    issuer: Option[str],
    timestamp: Option[str],
  }

  /// A verdict carries the note tag the reject site logs, so the outcome class survives
  /// into the observation layer.
  type SigVerdict = SigOk | SigBad(str)

  /// `validate_abi_signature` (abi_metadata.rs:268) and `validate_idl_signature`
  /// (idl/signature.rs:~150) in the exact order they check: algorithm present, algorithm
  /// supported, public key present, signature decodes, public key decodes, signature
  /// verifies over the recomputed prehash, signer in the allowlist.
  pure def checkSignature(
    sig: SigMeta, expectedAlg: str, expectedPrehash: Bytes, allow: Set[str]
  ): SigVerdict =
    match sig.algorithm {
      | AlgMissing => SigBad("signature_rejected_missing_algorithm")
      | AlgIs(alg) =>
          if (alg != expectedAlg) SigBad("signature_rejected_unsupported_algorithm")
          else
            match sig.publicKey {
              | KeyMissing => SigBad("signature_rejected_missing_public_key")
              | KeyMalformed =>
                  match sig.value {
                    | SigMalformed => SigBad("signature_rejected_invalid_signature_encoding")
                    | SigOver(_) => SigBad("signature_rejected_invalid_public_key")
                  }
              | KeyIs(key) =>
                  match sig.value {
                    | SigMalformed => SigBad("signature_rejected_invalid_signature_encoding")
                    | SigOver(att) =>
                        if (att.signer != key or att.prehash != expectedPrehash)
                          SigBad("signature_rejected_verification_failed")
                        else if (not(allow.contains(key)))
                          SigBad("signature_rejected_not_in_allowlist")
                        else SigOk
                  }
            }
    }

  /// Fields an admitted entry's signature metadata carries that neither the prehash
  /// covers nor any validation step reads. `issuer` and `timestamp` are copied out of the
  /// proto by `convert_proto_signature` into `#[allow(dead_code)]` fields.
  pure def unverifiedFieldsPresent(sig: SigMeta): Set[str] = {
    val carried =
      (if (sig.issuer != None) Set("issuer") else Set())
        .union(if (sig.timestamp != None) Set("timestamp") else Set())
    carried.exclude(PREHASH_COVERED_FIELDS.union(SIGNATURE_FIELDS_READ_BY_VALIDATION))
  }

  /// The five ways `SignatureMetadata` can be defective before the signature maths is
  /// even reached. Each is a distinct reject site in the verifier.
  pure val SIG_DEFECTS: Set[str] = Set(
    "no_algorithm", "wrong_algorithm", "no_public_key",
    "malformed_signature", "malformed_public_key",
  )

  pure def defectiveSig(defect: str, alg: str, signer: str, prehash: Bytes): SigMeta = {
    val att = SigOver({ prehash: prehash, signer: signer })
    if (defect == "no_algorithm")
      { value: att, algorithm: AlgMissing, publicKey: KeyIs(signer),
        issuer: None, timestamp: None }
    else if (defect == "wrong_algorithm")
      { value: att, algorithm: AlgIs("rsa"), publicKey: KeyIs(signer),
        issuer: None, timestamp: None }
    else if (defect == "no_public_key")
      { value: att, algorithm: AlgIs(alg), publicKey: KeyMissing,
        issuer: None, timestamp: None }
    else if (defect == "malformed_signature")
      { value: SigMalformed, algorithm: AlgIs(alg), publicKey: KeyIs(signer),
        issuer: None, timestamp: None }
    else
      { value: att, algorithm: AlgIs(alg), publicKey: KeyMalformed,
        issuer: None, timestamp: None }
  }

  // ===========================================================================
  // Ethereum world
  // ===========================================================================

  /// A signature outcome no validation step of the model can decide for itself.
  pure val VERDICT_UNREPORTED: str = "unreported"
  pure val VERDICT_ACCEPTED: str = "accepted"

  /// Every outcome the verifier can report for a present signature.
  pure val SIG_VERDICTS: Set[str] = SIG_CAUSES.union(Set(VERDICT_ACCEPTED))

  /// One `(map key, Abi)` pair of the caller's `abi_mappings`.
  ///
  /// `sigVerdict` is what the real verifier answered about this entry's signature, once it
  /// has run. Whether an ECDSA signature verifies, and whether its signer is on the curator
  /// allowlist, are decisions only the parser can make -- no logged fact can tell the model
  /// what a signature was minted over. While it is `VERDICT_UNREPORTED` (every simulated
  /// entry) the verdict is derived from `signature` by `checkSignature` instead, which is
  /// what keeps the adversary model's fidelity.
  ///
  /// `bodyParses` is the same arrangement for `serde_json::from_str::<JsonAbi>`: whether a
  /// caller's ABI JSON parses is the parser's answer, reported once `register_abi` has
  /// tried it. `None` means it has not run, and `abiParseable` stands in for it.
  type AbiEntry = {
    body: str,
    abiType: str,
    implAddressKey: Option[str],
    signature: Option[SigMeta],
    sigVerdict: str,
    bodyParses: Option[bool],
  }

  /// `AddressMapping` (abi_registry.rs:~36) plus two ghosts: the body a curator signature
  /// vouched for at admission, and the caller map key that installed the mapping.
  type EthMapping = {
    abiName: str,
    abiKind: str,
    implementation: Option[str],
    vouchedBody: Option[str],
    vouchedSigner: Option[str],
    installedByMetadataKey: Option[str],
  }

  /// The public key an admitted entry's signature was verified against -- the key whose
  /// presence in the allowlist is the whole reason the entry was admitted.
  pure def ethVouchedSigner(entry: AbiEntry): Option[str] =
    match entry.signature {
      | None => None
      | Some(sig) =>
          match sig.publicKey {
            | KeyIs(k) => Some(k)
            | KeyMissing => None
            | KeyMalformed => None
          }
    }

  /// `AbiRegistry` (abi_registry.rs:~63). `abis` is keyed by the RAW caller map key,
  /// because that is what `register_embedded_abi(&mut registry, address, ..)` passes as
  /// the ABI name. `firstMetadataInstaller` is a ghost recording which caller key first
  /// installed each parsed address.
  type EthRegistry = {
    abis: str -> str,
    mappings: (int, str) -> EthMapping,
    firstMetadataInstaller: (int, str) -> str,
  }

  pure val EMPTY_ETH_REGISTRY: EthRegistry =
    { abis: Map(), mappings: Map(), firstMetadataInstaller: Map() }

  type EthWorld = {
    signers: SignerConfig,
    chainId: int,
    extracting: bool,
    pending: str -> AbiEntry,
    processed: Set[str],
    admittedKeys: Set[str],
    rejectedKeys: Set[str],
    ambiguousKeys: Set[str],
    admittedBodies: Set[str],
    unsignedCount: int,
    notes: Set[str],
    registry: EthRegistry,
    returnedRegistry: bool,
  }

  pure val IDLE_ETH: EthWorld = {
    signers: UNBUILT_SIGNERS,
    chainId: 1,
    extracting: false,
    pending: Map(),
    processed: Set(),
    admittedKeys: Set(),
    rejectedKeys: Set(),
    ambiguousKeys: Set(),
    admittedBodies: Set(),
    unsignedCount: 0,
    notes: Set(),
    registry: EMPTY_ETH_REGISTRY,
    returnedRegistry: false,
  }

  /// Outcome classes of one iteration of the `try_extract_from_chain_metadata` loop.
  type EthVerdict =
    | EthInvalidAddress
    | EthOversized
    | EthBadSignature(str)
    | EthUnparseableAbi
    | EthAdmit

  pure def ethEntryVerdict(w: EthWorld, mapKey: str): EthVerdict = {
    val entry = w.pending.get(mapKey)
    match parseEthAddr(mapKey) {
      | None => EthInvalidAddress
      | Some(addr) =>
          if (abiOversized(entry.body)) EthOversized
          else {
            val sigVerdict =
              if (entry.sigVerdict == VERDICT_ACCEPTED) SigOk
              else if (entry.sigVerdict != VERDICT_UNREPORTED) SigBad(entry.sigVerdict)
              else
                match entry.signature {
                  | None => SigOk
                  | Some(sig) =>
                      checkSignature(sig, "secp256k1",
                        ethPrehash(w.chainId, addr, abiBodyBytes(entry.body)), w.signers.keys)
                }
            match sigVerdict {
              | SigBad(cause) => EthBadSignature(cause)
              | SigOk => {
                  val parses =
                    match entry.bodyParses {
                      | Some(reported) => reported
                      | None => abiParseable(entry.body)
                    }
                  if (parses) EthAdmit else EthUnparseableAbi
                }
            }
          }
    }
  }

  /// True when the verifier refused this entry's signature, whichever check refused it.
  /// WHICH check fails is the parser's answer, not the model's: `checkSignature` stands in
  /// for real ECDSA verification and allowlist membership during simulation, and no logged
  /// fact can tell the model what a signature was minted over. The precedence this guard
  /// does carry is the valuable part -- a signature verdict is only reached once the
  /// address parsed and the body was within the size bound.
  pure def ethVerdictIsBadSignature(w: EthWorld, mapKey: str): bool =
    match ethEntryVerdict(w, mapKey) {
      | EthBadSignature(_) => true
      | _ => false
    }

  /// `resolve_abi_kind` (abi_metadata.rs:~165). Unset and out-of-enum wire values both
  /// collapse to Implementation.
  pure def ethResolveKind(entry: AbiEntry): str =
    if (entry.abiType == "proxy") "proxy" else "implementation"

  /// The proxy's `implementation_address`, parsed best-effort. Missing or malformed
  /// yields no link -- the entry is kept, not dropped.
  pure def ethResolveImpl(entry: AbiEntry): Option[str] =
    if (entry.abiType != "proxy") None
    else match entry.implAddressKey {
      | None => None
      | Some(raw) => parseEthAddr(raw)
    }

  /// Log sites `resolve_abi_kind` and the unsigned-acceptance path reach without changing
  /// any observable field -- latched so the outcome class stays visible.
  pure def ethAdmitNotes(entry: AbiEntry): Set[str] = {
    val unsignedNote = if (entry.signature == None) Set("accepted_unsigned") else Set()
    if (entry.abiType != "proxy") unsignedNote
    else
      match entry.implAddressKey {
        | None => unsignedNote.union(Set("proxy_implementation_address_missing"))
        | Some(raw) =>
            if (parseEthAddr(raw) == None)
              unsignedNote.union(Set("proxy_implementation_address_malformed"))
            else unsignedNote
      }
  }

  /// `get_abi_for_address` (abi_registry.rs:~160): the ABI body registered for a mapped
  /// address, if the name it maps to was ever registered.
  pure def ethOwnBody(reg: EthRegistry, chainId: int, addr: str): Option[str] =
    if (not(reg.mappings.has((chainId, addr)))) None
    else {
      val name = reg.mappings.get((chainId, addr)).abiName
      if (reg.abis.has(name)) Some(reg.abis.get(name)) else None
    }

  // ===========================================================================
  // Solana world
  // ===========================================================================

  /// One `(program id, Idl)` pair of the caller's `idl_mappings`. `sigVerdict` is the
  /// verifier's answer about this entry's ed25519 signature; see `AbiEntry`.
  type IdlEntry = {
    body: str,
    programName: Option[str],
    signature: Option[SigMeta],
    sigVerdict: str,
    bodyOversized: Option[bool],
  }

  /// One surviving row of `extract_idl_mappings`: `(program_id -> (idl_json, name))`,
  /// plus a ghost recording whether a curator signature carried it in.
  type ExtractedIdl = { body: str, name: str, signed: bool }

  /// `IdlRegistry` (idl/mod.rs:~23). `idlNames` is populated by a SECOND, independent
  /// extraction of `metadata.name` that applies no reserved-name filter.
  type IdlRegistry = { configs: str -> str, names: str -> str, idlNames: str -> str }

  pure val EMPTY_IDL_REGISTRY: IdlRegistry =
    { configs: Map(), names: Map(), idlNames: Map() }

  type SolWorld = {
    signers: SignerConfig,
    extracting: bool,
    pending: str -> IdlEntry,
    processed: Set[str],
    extracted: str -> ExtractedIdl,
    rejectedKeys: Set[str],
    ambiguousKeys: Set[str],
    notes: Set[str],
    registry: IdlRegistry,
    requestOk: bool,
  }

  pure val IDLE_SOL: SolWorld = {
    signers: UNBUILT_SIGNERS,
    extracting: false,
    pending: Map(),
    processed: Set(),
    extracted: Map(),
    rejectedKeys: Set(),
    ambiguousKeys: Set(),
    notes: Set(),
    registry: EMPTY_IDL_REGISTRY,
    requestOk: false,
  }

  type SolVerdict =
    | SolInvalidProgramId
    | SolTrustedProgram
    | SolOversized
    | SolBadSignature(str)
    | SolReservedName
    | SolAccept

  /// The name the impersonation guard actually tests: `program_name` when present,
  /// otherwise `metadata.name` from the body, otherwise the id-derived fallback.
  pure def solResolvedName(entry: IdlEntry): str =
    match entry.programName {
      | Some(n) => n
      | None =>
          match idlMetadataName(entry.body) {
            | Some(n) => n
            | None => SOL_FALLBACK_NAME
          }
    }

  pure def solEntryVerdict(w: SolWorld, programId: str): SolVerdict = {
    val entry = w.pending.get(programId)
    if (not(solParsePubkey(programId))) SolInvalidProgramId
    else if (solIsTrusted(programId)) SolTrustedProgram
    else if (match entry.bodyOversized {
               | Some(reported) => reported
               | None => idlOversized(entry.body)
             }) SolOversized
    else {
      val sigVerdict =
        if (entry.sigVerdict == VERDICT_ACCEPTED) SigOk
        else if (entry.sigVerdict != VERDICT_UNREPORTED) SigBad(entry.sigVerdict)
        else
          match entry.signature {
            | None => SigOk
            | Some(sig) =>
                checkSignature(sig, "ed25519",
                  solPrehash(programId, idlBodyBytes(entry.body)), w.signers.keys)
          }
      match sigVerdict {
        | SigBad(cause) => SolBadSignature(cause)
        | SigOk =>
            if (RESERVED_CANONICAL_NAMES.contains(solResolvedName(entry))) SolReservedName
            else SolAccept
      }
    }
  }

  /// The Solana twin: the verifier refused the signature, whichever check refused it.
  pure def solVerdictIsBadSignature(w: SolWorld, programId: str): bool =
    match solEntryVerdict(w, programId) {
      | SolBadSignature(_) => true
      | _ => false
    }

  pure def solMetaNameOr(body: str, fallback: str): str =
    match idlMetadataName(body) { | Some(n) => n | None => fallback }

  /// `IdlRegistry::from_idl_mappings` (idl/mod.rs:66). Re-applies the trusted filter, then
  /// stores the body UNVALIDATED (a `serde_json` failure on the metadata read is
  /// discarded, and nothing else ever parses the body) with `override_builtin = true`.
  pure def solBuildRegistry(extracted: str -> ExtractedIdl): IdlRegistry = {
    val kept = extracted.keys().filter(p => not(solIsTrusted(p)))
    { configs: kept.mapBy(p => extracted.get(p).body),
      names: kept.mapBy(p => extracted.get(p).name),
      idlNames: kept.filter(p => idlMetadataName(extracted.get(p).body) != None)
                    .mapBy(p => solMetaNameOr(extracted.get(p).body, SOL_FALLBACK_NAME)) }
  }

  // ===========================================================================
  // Guarded reads
  // ===========================================================================

  type ReadOutcome =
    | NoRead
    | EthAbiFound(str)
    | EthAbiMissing
    | EthKindRead(str)
    | EthKindUnmapped
    | EthImplResolved({ impl: str, body: str })
    | EthImplUnresolved(str)
    | EthListing(int)
    | EthNamedAbiFound(str)
    | EthNamedAbiMissing
    | IdlAdvertised(str)
    | IdlNotAdvertised
    | IdlDecoded(str)
    | IdlUndecodable
    | IdlAbsent
    | ProgramNameCanonical(str)
    | ProgramNameCaller(str)
    | ProgramNameFallback
    | IdlNameSuppressed
    | IdlNameShown(str)
    | IdlNameAbsent

  /// `get_implementation_abi` (abi_registry.rs:~178). Four distinct unresolved causes,
  /// each of which sends the caller back to the proxy's own ABI (lib.rs:583).
  pure def ethImplRead(reg: EthRegistry, chainId: int, addr: str): ReadOutcome =
    if (not(reg.mappings.has((chainId, addr)))) EthImplUnresolved("not_mapped")
    else {
      val m = reg.mappings.get((chainId, addr))
      if (m.abiKind != "proxy") EthImplUnresolved("not_proxy")
      else
        match m.implementation {
          | None => EthImplUnresolved("no_link")
          | Some(target) =>
              match ethOwnBody(reg, chainId, target) {
                | None => EthImplUnresolved("implementation_unregistered")
                | Some(b) => EthImplResolved({ impl: target, body: b })
              }
        }
    }

  /// The ABI body that actually renders calldata sent to `addr`
  /// (`visualize_with_abi_registry`, lib.rs:583): the linked implementation's body when
  /// the proxy link resolves, otherwise the address's own ABI.
  pure def ethDecoderBody(reg: EthRegistry, chainId: int, addr: str): Option[str] =
    match ethImplRead(reg, chainId, addr) {
      | EthImplResolved(r) => Some(r.body)
      | _ => ethOwnBody(reg, chainId, addr)
    }

  /// `IdlRegistry::get_program_name` (idl/mod.rs:~140).
  pure def solProgramNameRead(reg: IdlRegistry, programId: str): ReadOutcome =
    match solCanonicalName(programId) {
      | Some(cn) => ProgramNameCanonical(cn)
      | None =>
          if (reg.names.has(programId)) ProgramNameCaller(reg.names.get(programId))
          else ProgramNameFallback
    }

  /// `IdlRegistry::get_idl_name` (idl/mod.rs:~168). Suppression is keyed on the PROGRAM
  /// having a canonical identity, never on the name being a reserved one.
  pure def solIdlNameRead(reg: IdlRegistry, programId: str): ReadOutcome =
    if (solCanonicalName(programId) != None) IdlNameSuppressed
    else if (reg.idlNames.has(programId)) IdlNameShown(reg.idlNames.get(programId))
    else IdlNameAbsent

  // ===========================================================================
  // State
  // ===========================================================================

  /// One recorded `metadata_signing_prehash_v1` call: the fields that went in and the
  /// preimage that came out.
  type PrehashInput = { tag: Bytes, scope: Bytes, body: Bytes }

  /// Probe inputs for the injectivity question, chosen to include pairs whose bytes
  /// differ only in WHERE the boundary between adjacent fields falls -- exactly what an
  /// unprefixed concatenation would collapse.
  pure val PREHASH_PROBES: Set[PrehashInput] = Set(
    { tag: TAG_ETHEREUM, scope: List(1, 11), body: List(21) },
    { tag: TAG_ETHEREUM, scope: List(1), body: List(11, 21) },
    { tag: TAG_ETHEREUM, scope: List(1, 11), body: List(22) },
    { tag: TAG_ETHEREUM, scope: List(2, 11), body: List(21) },
    { tag: TAG_SOLANA, scope: List(44), body: List(31) },
    { tag: TAG_SOLANA, scope: List(), body: List(44, 31) },
  )

  var eth: EthWorld
  var sol: SolWorld
  var prehashLedger: Set[{ input: PrehashInput, digest: Bytes }]
  var lastRead: ReadOutcome

  action init: bool = all {
    eth' = IDLE_ETH,
    sol' = IDLE_SOL,
    prehashLedger' = Set(),
    lastRead' = NoRead,
  }

  // ===========================================================================
  // Actions -- shared prehash and allowlist (visualsign/src/signing.rs)
  // ===========================================================================

  /// `metadata_signing_prehash_v1(chain_tag, scope, body)`.
  action compute_metadata_prehash(probe: PrehashInput): bool = all {
    prehashLedger' = prehashLedger.union(
      Set({ input: probe, digest: prehashV1(probe.tag, probe.scope, probe.body) })),
    eth' = eth, sol' = sol, lastRead' = lastRead,
  }

  /// `SignerAllowlist::insert` used directly, outside either `authorized_*_signers`
  /// builder -- any embedder holding a `SignerAllowlist` can do this.
  action eth_allowlist_insert_key(key: str): bool = all {
    not(eth.extracting),
    eth' = { ...eth, signers: { ...eth.signers, keys: eth.signers.keys.union(Set(key)),
                                built: true } },
    sol' = sol, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  action sol_allowlist_insert_key(key: str): bool = all {
    not(sol.extracting),
    sol' = { ...sol, signers: { ...sol.signers, keys: sol.signers.keys.union(Set(key)),
                                built: true } },
    eth' = eth, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  /// `SignerAllowlist::from_iter` -- the bulk constructor, which REPLACES the whole
  /// allowlist rather than extending it.
  action eth_allowlist_from_keys(keySet: Set[str]): bool = all {
    not(eth.extracting),
    eth' = { ...eth, signers: { ...eth.signers, keys: keySet, built: true } },
    sol' = sol, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  action sol_allowlist_from_keys(keySet: Set[str]): bool = all {
    not(sol.extracting),
    sol' = { ...sol, signers: { ...sol.signers, keys: keySet, built: true } },
    eth' = eth, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  // ===========================================================================
  // Actions -- Ethereum ABI admission (abi_metadata.rs)
  // ===========================================================================

  /// The deployment sets `VISUALSIGN_ETH_ABI_SIGNERS`.
  action set_eth_signer_env(entries: List[str]): bool = all {
    not(eth.extracting),
    eth' = { ...eth, signers: { ...eth.signers, env: entries } },
    sol' = sol, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  /// `authorized_abi_signers` (abi_metadata.rs:351). Rebuilt on every call. Malformed env
  /// entries are logged and skipped; `devSigning` adds the compile-time dev key, which
  /// only test and `dev-signing` builds have.
  action build_eth_signer_allowlist(devSigning: bool): bool = all {
    not(eth.extracting),
    eth' = { ...eth,
      signers: {
        env: eth.signers.env,
        keys: envKeys(eth.signers.env).union(if (devSigning) Set(DEV_KEY) else Set()),
        dropped: envDropped(eth.signers.env),
        built: true,
        builtFromEnv: eth.signers.env,
      },
      notes: if (envDropped(eth.signers.env) > 0)
               eth.notes.union(Set("allowlist_dropped_invalid_key")) else eth.notes,
    },
    sol' = sol, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  /// `try_extract_from_chain_metadata` starts: a fresh `AbiRegistry::new()` for the
  /// request, with the resolved chain id the prehash scope will commit to.
  action begin_eth_extraction(chainId: int): bool = all {
    not(eth.extracting),
    eth.pending.keys().size() > 0,
    eth' = { ...eth, extracting: true, chainId: chainId, processed: Set(),
             admittedKeys: Set(), rejectedKeys: Set(), admittedBodies: Set(),
             unsignedCount: 0, registry: EMPTY_ETH_REGISTRY, returnedRegistry: false },
    sol' = sol, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  /// The number of entries one request's caller mapping is modelled as carrying. Two is
  /// enough for both multi-entry behaviours the code has (an alias pair, and a proxy plus
  /// its implementation); the real bound is the 1 MB per-entry size cap, not a count.
  pure val MAX_MODELLED_MAPPING_ENTRIES: int = 3

  /// The caller puts an entry into `ChainMetadata.ethereum.abi_mappings`.
  action offer_eth_abi_entry(mapKey: str, body: str): bool = all {
    not(eth.extracting),
    eth.pending.has(mapKey) or eth.pending.keys().size() < MAX_MODELLED_MAPPING_ENTRIES,
    eth' = { ...eth, pending: eth.pending.put(mapKey,
      { body: body, abiType: "unset", implAddressKey: None, signature: None,
        sigVerdict: VERDICT_UNREPORTED, bodyParses: None }) },
    sol' = sol, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  /// `abi_type` and `implementation_address`. Deliberately a separate transition: neither
  /// field is covered by the ABI signature, so anyone on the path can set them
  /// independently of who signed the body.
  action set_eth_entry_routing(mapKey: str, abiType: str, implKey: str): bool = all {
    not(eth.extracting),
    eth.pending.has(mapKey),
    eth' = { ...eth, pending: eth.pending.put(mapKey,
      { ...eth.pending.get(mapKey), abiType: abiType,
        implAddressKey: if (implKey == NO_IMPL_ADDRESS) None else Some(implKey) }) },
    sol' = sol, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  /// `sign_abi` (abi_metadata.rs:~410) mints a signature over the prehash for a
  /// (chain id, address) of the signer's choosing -- which need not be the entry's own.
  action attach_eth_signature(
    mapKey: str, signerKey: str, boundKey: str, boundChainId: int
  ): bool = all {
    not(eth.extracting),
    eth.pending.has(mapKey),
    eth' = { ...eth, ambiguousKeys: eth.ambiguousKeys.exclude(Set(mapKey)),
      pending: eth.pending.put(mapKey, {
      ...eth.pending.get(mapKey),
      signature: Some({
        value: SigOver({
          prehash: ethPrehash(boundChainId, ethAddrOr(boundKey, "addrA"),
                              abiBodyBytes(eth.pending.get(mapKey).body)),
          signer: signerKey }),
        algorithm: AlgIs("secp256k1"),
        publicKey: KeyIs(signerKey),
        issuer: None,
        timestamp: None }) }) },
    sol' = sol, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  /// A `SignatureMetadata` that is defective before the maths: no algorithm, an
  /// unsupported one, no public key, undecodable signature bytes, undecodable key bytes.
  action attach_defective_eth_signature(mapKey: str, defect: str): bool = all {
    not(eth.extracting),
    eth.pending.has(mapKey),
    eth' = { ...eth, ambiguousKeys: eth.ambiguousKeys.exclude(Set(mapKey)),
      pending: eth.pending.put(mapKey, {
      ...eth.pending.get(mapKey),
      signature: Some(defectiveSig(defect, "secp256k1", "curator",
        ethPrehash(eth.chainId, ethAddrOr(mapKey, "addrA"),
                   abiBodyBytes(eth.pending.get(mapKey).body)))) }) },
    sol' = sol, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  /// `convert_proto_signature` copies `issuer` and `timestamp` out of the proto metadata
  /// map. Neither is in the prehash and neither is read again.
  action set_eth_signature_provenance(mapKey: str): bool = all {
    not(eth.extracting),
    eth.pending.has(mapKey),
    eth.pending.get(mapKey).signature != None,
    eth' = { ...eth, pending: eth.pending.put(mapKey, {
      ...eth.pending.get(mapKey),
      signature: match eth.pending.get(mapKey).signature {
        | None => None
        | Some(s) => Some({ ...s, issuer: Some("acme-curators"),
                                  timestamp: Some("2024-01") }) } }) },
    sol' = sol, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  /// `convert_proto_signature` (abi_metadata.rs:206) reads each key with
  /// `.find(|m| m.key == key)`, so a proto carrying TWO `algorithm` or `public_key` pairs
  /// resolves to whichever comes FIRST in the list. The second value is silently dropped
  /// rather than the entry being rejected as ambiguous, so the caller chooses which of two
  /// declared keys the entry is admitted on.
  action attach_eth_signature_with_shadowed_metadata(
    mapKey: str, firstKey: str, shadowedKey: str
  ): bool = all {
    not(eth.extracting),
    eth.pending.has(mapKey),
    firstKey != shadowedKey,
    eth' = { ...eth,
      ambiguousKeys: eth.ambiguousKeys.union(Set(mapKey)),
      notes: eth.notes.union(Set("signature_metadata_duplicate_resolved_by_list_order")),
      pending: eth.pending.put(mapKey, {
        ...eth.pending.get(mapKey),
        signature: Some({
          value: SigOver({
            prehash: ethPrehash(eth.chainId, ethAddrOr(mapKey, "addrA"),
                                abiBodyBytes(eth.pending.get(mapKey).body)),
            signer: firstKey }),
          algorithm: AlgIs("secp256k1"),
          publicKey: KeyIs(firstKey),
          issuer: None,
          timestamp: None }) }) },
    sol' = sol, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  /// Anyone who can reach the metadata can simply delete the signature field.
  action strip_eth_signature(mapKey: str): bool = all {
    not(eth.extracting),
    eth.pending.has(mapKey),
    eth.pending.get(mapKey).signature != None,
    eth' = { ...eth, ambiguousKeys: eth.ambiguousKeys.exclude(Set(mapKey)),
      pending: eth.pending.put(mapKey,
      { ...eth.pending.get(mapKey), signature: None,
        sigVerdict: VERDICT_UNREPORTED }) },
    sol' = sol, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  /// The body is changed after signing; the signature is left in place.
  action tamper_eth_abi_body(mapKey: str, body: str): bool = all {
    not(eth.extracting),
    eth.pending.has(mapKey),
    eth.pending.get(mapKey).body != body,
    eth' = { ...eth, pending: eth.pending.put(mapKey,
      { ...eth.pending.get(mapKey), body: body }) },
    sol' = sol, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  /// `register_abi` has tried to parse this entry's ABI JSON and this is what it answered.
  action report_eth_abi_body_parse(mapKey: str, parses: bool): bool = all {
    eth.extracting,
    eth.pending.has(mapKey),
    not(eth.processed.contains(mapKey)),
    eth' = { ...eth, pending: eth.pending.put(mapKey,
      { ...eth.pending.get(mapKey), bodyParses: Some(parses) }) },
    sol' = sol, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  /// `validate_abi_signature` has run and this is what it answered. Reported rather than
  /// re-derived, for the reason on `AbiEntry.sigVerdict`.
  action report_eth_signature_verdict(mapKey: str, verdict: str): bool = all {
    eth.extracting,
    eth.pending.has(mapKey),
    not(eth.processed.contains(mapKey)),
    eth.pending.get(mapKey).signature != None,
    SIG_VERDICTS.contains(verdict),
    eth' = { ...eth, pending: eth.pending.put(mapKey,
      { ...eth.pending.get(mapKey), sigVerdict: verdict }) },
    sol' = sol, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  pure def ethMarkRejected(w: EthWorld, mapKey: str, note: str): EthWorld =
    { ...w, processed: w.processed.union(Set(mapKey)),
            rejectedKeys: w.rejectedKeys.union(Set(mapKey)),
            notes: w.notes.union(Set(note)) }

  action eth_skip_invalid_address(mapKey: str): bool = all {
    eth.extracting,
    eth.pending.has(mapKey),
    not(eth.processed.contains(mapKey)),
    ethEntryVerdict(eth, mapKey) == EthInvalidAddress,
    eth' = ethMarkRejected(eth, mapKey, "skip_invalid_address"),
    sol' = sol, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  action eth_skip_oversized_body(mapKey: str): bool = all {
    eth.extracting,
    eth.pending.has(mapKey),
    not(eth.processed.contains(mapKey)),
    ethEntryVerdict(eth, mapKey) == EthOversized,
    eth' = ethMarkRejected(eth, mapKey, "skip_oversized_body"),
    sol' = sol, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  action eth_skip_invalid_signature(mapKey: str, cause: str): bool = all {
    eth.extracting,
    eth.pending.has(mapKey),
    not(eth.processed.contains(mapKey)),
    SIG_CAUSES.contains(cause),
    ethVerdictIsBadSignature(eth, mapKey),
    eth' = ethMarkRejected(eth, mapKey, cause),
    sol' = sol, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  action eth_skip_unparseable_abi_json(mapKey: str): bool = all {
    eth.extracting,
    eth.pending.has(mapKey),
    not(eth.processed.contains(mapKey)),
    ethEntryVerdict(eth, mapKey) == EthUnparseableAbi,
    eth' = ethMarkRejected(eth, mapKey, "skip_unparseable_abi_json"),
    sol' = sol, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  pure def ethApplyAdmit(w: EthWorld, mapKey: str): EthWorld = {
    val entry = w.pending.get(mapKey)
    val addr = ethAddrOr(mapKey, "addrA")
    val slot = (w.chainId, addr)
    val reg = w.registry
    { ...w,
      processed: w.processed.union(Set(mapKey)),
      admittedKeys: w.admittedKeys.union(Set(mapKey)),
      admittedBodies: w.admittedBodies.union(Set(entry.body)),
      unsignedCount:
        if (entry.signature == None) w.unsignedCount + 1 else w.unsignedCount,
      notes: w.notes.union(ethAdmitNotes(entry)),
      registry: {
        abis: reg.abis.put(mapKey, entry.body),
        mappings: reg.mappings.put(slot, {
          abiName: mapKey,
          abiKind: ethResolveKind(entry),
          implementation: ethResolveImpl(entry),
          vouchedBody: if (entry.signature == None) None else Some(entry.body),
          vouchedSigner: ethVouchedSigner(entry),
          installedByMetadataKey: Some(mapKey) }),
        firstMetadataInstaller:
          if (reg.firstMetadataInstaller.has(slot)) reg.firstMetadataInstaller
          else reg.firstMetadataInstaller.put(slot, mapKey),
      },
    }
  }

  /// `register_embedded_abi` under the RAW map key, then `map_address_with_type` under
  /// the PARSED address -- the two different keys that make aliasing possible.
  action eth_admit_entry(mapKey: str): bool = all {
    eth.extracting,
    eth.pending.has(mapKey),
    not(eth.processed.contains(mapKey)),
    ethEntryVerdict(eth, mapKey) == EthAdmit,
    eth' = ethApplyAdmit(eth, mapKey),
    sol' = sol, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  /// The loop ends. `list_abis().is_empty()` decides whether a registry is returned at all.
  action finish_eth_extraction: bool = all {
    eth.extracting,
    eth.pending.keys().exclude(eth.processed) == Set(),
    // The mapping belonged to this request: a later call carries its own.
    eth' = { ...eth, extracting: false, pending: Map(), processed: Set(),
             returnedRegistry: eth.registry.abis.keys().size() > 0,
             notes: if (eth.registry.abis.keys().size() == 0)
                      eth.notes.union(Set("extraction_returned_no_registry"))
                    else eth.notes },
    sol' = sol, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  // ===========================================================================
  // Actions -- direct AbiRegistry API (abi_registry.rs)
  // ===========================================================================

  /// ABI names the compile-time/embedded path uses. `Unregistered` is never registered.
  pure val ETH_ABI_NAMES: Set[str] = Set("EmbeddedToken", "Unregistered")

  /// `AbiRegistry::register_abi` -- parses the JSON, errors when it does not parse. No
  /// size bound on this path.
  action registry_register_abi(name: str, body: str): bool = all {
    not(eth.extracting),
    abiParseable(body),
    eth' = { ...eth, registry: { ...eth.registry, abis: eth.registry.abis.put(name, body) } },
    sol' = sol, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  /// `AbiRegistry::map_address` -- always Implementation, never a proxy link.
  action registry_map_address(chainId: int, addr: str, abiName: str): bool = all {
    not(eth.extracting),
    eth' = { ...eth, registry: { ...eth.registry,
      mappings: eth.registry.mappings.put((chainId, addr), {
        abiName: abiName, abiKind: "implementation", implementation: None,
        vouchedBody: None, vouchedSigner: None, installedByMetadataKey: None }) } },
    sol' = sol, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  /// `AbiRegistry::map_address_with_type` -- the unchecked writer. It never verifies that
  /// `abi_name` was registered, and never verifies that a proxy's `implementation` is
  /// mapped either.
  action registry_map_address_with_type(
    chainId: int, addr: str, abiName: str, kind: str, impl: Option[str]
  ): bool = all {
    not(eth.extracting),
    eth' = { ...eth, registry: { ...eth.registry,
      mappings: eth.registry.mappings.put((chainId, addr), {
        abiName: abiName, abiKind: kind, implementation: impl,
        vouchedBody: None, vouchedSigner: None, installedByMetadataKey: None }) } },
    sol' = sol, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  action read_abi_for_address(chainId: int, addr: str): bool = all {
    lastRead' = match ethOwnBody(eth.registry, chainId, addr) {
      | Some(b) => EthAbiFound(b)
      | None => EthAbiMissing
    },
    eth' = eth, sol' = sol, prehashLedger' = prehashLedger,
  }

  action read_abi_kind(chainId: int, addr: str): bool = all {
    lastRead' =
      if (eth.registry.mappings.has((chainId, addr)))
        EthKindRead(eth.registry.mappings.get((chainId, addr)).abiKind)
      else EthKindUnmapped,
    eth' = eth, sol' = sol, prehashLedger' = prehashLedger,
  }

  action read_implementation_abi(chainId: int, addr: str): bool = all {
    lastRead' = ethImplRead(eth.registry, chainId, addr),
    eth' = eth, sol' = sol, prehashLedger' = prehashLedger,
  }

  action read_abi_by_name(name: str): bool = all {
    lastRead' =
      if (eth.registry.abis.has(name)) EthNamedAbiFound(eth.registry.abis.get(name))
      else EthNamedAbiMissing,
    eth' = eth, sol' = sol, prehashLedger' = prehashLedger,
  }

  /// `list_abis` / `list_mappings_for_chain` -- the listing whose emptiness decides
  /// whether `try_extract_from_chain_metadata` returns a registry at all.
  action read_registry_listing(chainId: int): bool = all {
    lastRead' = EthListing(
      eth.registry.mappings.keys().filter(k => k._1 == chainId).size()),
    eth' = eth, sol' = sol, prehashLedger' = prehashLedger,
  }

  // ===========================================================================
  // Actions -- Solana IDL admission (core/visualsign.rs + idl/*)
  // ===========================================================================

  action set_sol_signer_env(entries: List[str]): bool = all {
    not(sol.extracting),
    sol' = { ...sol, signers: { ...sol.signers, env: entries } },
    eth' = eth, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  /// `authorized_idl_signers` (idl/signature.rs:~215). A `OnceLock`: the env is read, hex
  /// decoded and point-validated exactly ONCE per process, and the result is cached for
  /// the process lifetime. There is no dev key on this path.
  action build_sol_signer_allowlist: bool = all {
    not(sol.extracting),
    not(sol.signers.built),
    sol' = { ...sol,
      signers: {
        env: sol.signers.env,
        keys: envKeys(sol.signers.env),
        dropped: envDropped(sol.signers.env),
        built: true,
        builtFromEnv: sol.signers.env,
      },
      notes: if (envDropped(sol.signers.env) > 0)
               sol.notes.union(Set("allowlist_dropped_invalid_key")) else sol.notes,
    },
    eth' = eth, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  action begin_sol_extraction: bool = all {
    not(sol.extracting),
    sol.pending.keys().size() > 0,
    sol' = { ...sol, extracting: true, processed: Set(), extracted: Map(),
             rejectedKeys: Set(), registry: EMPTY_IDL_REGISTRY, requestOk: false },
    eth' = eth, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  action offer_idl_entry(programId: str, body: str): bool = all {
    not(sol.extracting),
    sol.pending.has(programId)
      or sol.pending.keys().size() < MAX_MODELLED_MAPPING_ENTRIES,
    sol' = { ...sol, pending: sol.pending.put(programId,
      { body: body, programName: None, signature: None,
        sigVerdict: VERDICT_UNREPORTED, bodyOversized: None }) },
    eth' = eth, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  /// The proto `Idl.program_name`. When set it wins over `metadata.name`, and it is the
  /// ONLY value the impersonation guard ever inspects.
  action set_idl_program_name(programId: str, name: str): bool = all {
    not(sol.extracting),
    sol.pending.has(programId),
    sol' = { ...sol, pending: sol.pending.put(programId,
      { ...sol.pending.get(programId), programName: Some(name) }) },
    eth' = eth, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  action attach_idl_signature(programId: str, signerKey: str, boundProgramId: str): bool = all {
    not(sol.extracting),
    sol.pending.has(programId),
    sol' = { ...sol, ambiguousKeys: sol.ambiguousKeys.exclude(Set(programId)),
      pending: sol.pending.put(programId, {
      ...sol.pending.get(programId),
      signature: Some({
        value: SigOver({
          prehash: solPrehash(boundProgramId, idlBodyBytes(sol.pending.get(programId).body)),
          signer: signerKey }),
        algorithm: AlgIs("ed25519"),
        publicKey: KeyIs(signerKey),
        issuer: None,
        timestamp: None }) }) },
    eth' = eth, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  action attach_defective_idl_signature(programId: str, defect: str): bool = all {
    not(sol.extracting),
    sol.pending.has(programId),
    sol' = { ...sol, ambiguousKeys: sol.ambiguousKeys.exclude(Set(programId)),
      pending: sol.pending.put(programId, {
      ...sol.pending.get(programId),
      signature: Some(defectiveSig(defect, "ed25519", "curator",
        solPrehash(programId, idlBodyBytes(sol.pending.get(programId).body)))) }) },
    eth' = eth, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  action set_idl_signature_provenance(programId: str): bool = all {
    not(sol.extracting),
    sol.pending.has(programId),
    sol.pending.get(programId).signature != None,
    sol' = { ...sol, pending: sol.pending.put(programId, {
      ...sol.pending.get(programId),
      signature: match sol.pending.get(programId).signature {
        | None => None
        | Some(s) => Some({ ...s, issuer: Some("acme-curators"),
                                  timestamp: Some("2024-01") }) } }) },
    eth' = eth, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  /// The Solana twin of the duplicate-metadata resolution (idl/signature.rs:93).
  action attach_idl_signature_with_shadowed_metadata(
    programId: str, firstKey: str, shadowedKey: str
  ): bool = all {
    not(sol.extracting),
    sol.pending.has(programId),
    firstKey != shadowedKey,
    sol' = { ...sol,
      ambiguousKeys: sol.ambiguousKeys.union(Set(programId)),
      notes: sol.notes.union(Set("signature_metadata_duplicate_resolved_by_list_order")),
      pending: sol.pending.put(programId, {
        ...sol.pending.get(programId),
        signature: Some({
          value: SigOver({
            prehash: solPrehash(programId, idlBodyBytes(sol.pending.get(programId).body)),
            signer: firstKey }),
          algorithm: AlgIs("ed25519"),
          publicKey: KeyIs(firstKey),
          issuer: None,
          timestamp: None }) }) },
    eth' = eth, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  action strip_idl_signature(programId: str): bool = all {
    not(sol.extracting),
    sol.pending.has(programId),
    sol.pending.get(programId).signature != None,
    sol' = { ...sol, ambiguousKeys: sol.ambiguousKeys.exclude(Set(programId)),
      pending: sol.pending.put(programId,
      { ...sol.pending.get(programId), signature: None,
        sigVerdict: VERDICT_UNREPORTED }) },
    eth' = eth, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  action tamper_idl_body(programId: str, body: str): bool = all {
    not(sol.extracting),
    sol.pending.has(programId),
    sol.pending.get(programId).body != body,
    sol' = { ...sol, pending: sol.pending.put(programId,
      { ...sol.pending.get(programId), body: body }) },
    eth' = eth, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  /// `extract_idl_mappings_with_signers` has returned: the caller's mapping has been
  /// walked. Building the registry is a separate function and a separate transition.
  action finish_sol_extraction: bool = all {
    sol.extracting,
    sol.pending.keys().exclude(sol.processed) == Set(),
    // The mapping belonged to this request: a later call carries its own.
    sol' = { ...sol, extracting: false, pending: Map(), processed: Set() },
    eth' = eth, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  /// The caller-supplied byte length of an IDL body, measured before the loop runs.
  action report_idl_body_size(programId: str, oversized: bool): bool = all {
    not(sol.extracting),
    sol.pending.has(programId),
    sol' = { ...sol, pending: sol.pending.put(programId,
      { ...sol.pending.get(programId), bodyOversized: Some(oversized) }) },
    eth' = eth, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  /// `validate_idl_signature` has run and this is what it answered.
  action report_idl_signature_verdict(programId: str, verdict: str): bool = all {
    sol.extracting,
    sol.pending.has(programId),
    not(sol.processed.contains(programId)),
    sol.pending.get(programId).signature != None,
    SIG_VERDICTS.contains(verdict),
    sol' = { ...sol, pending: sol.pending.put(programId,
      { ...sol.pending.get(programId), sigVerdict: verdict }) },
    eth' = eth, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  pure def solMarkRejected(w: SolWorld, programId: str, note: str): SolWorld =
    { ...w, processed: w.processed.union(Set(programId)),
            rejectedKeys: w.rejectedKeys.union(Set(programId)),
            notes: w.notes.union(Set(note)) }

  action sol_skip_invalid_program_id(programId: str): bool = all {
    sol.extracting,
    sol.pending.has(programId),
    not(sol.processed.contains(programId)),
    solEntryVerdict(sol, programId) == SolInvalidProgramId,
    sol' = solMarkRejected(sol, programId, "skip_invalid_program_id"),
    eth' = eth, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  action sol_skip_trusted_program(programId: str): bool = all {
    sol.extracting,
    sol.pending.has(programId),
    not(sol.processed.contains(programId)),
    solEntryVerdict(sol, programId) == SolTrustedProgram,
    sol' = solMarkRejected(sol, programId, "skip_trusted_program"),
    eth' = eth, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  action sol_skip_oversized_body(programId: str): bool = all {
    sol.extracting,
    sol.pending.has(programId),
    not(sol.processed.contains(programId)),
    solEntryVerdict(sol, programId) == SolOversized,
    sol' = solMarkRejected(sol, programId, "skip_oversized_body"),
    eth' = eth, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  action sol_skip_invalid_signature(programId: str, cause: str): bool = all {
    sol.extracting,
    sol.pending.has(programId),
    not(sol.processed.contains(programId)),
    SIG_CAUSES.contains(cause),
    solVerdictIsBadSignature(sol, programId),
    sol' = solMarkRejected(sol, programId, cause),
    eth' = eth, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  action sol_skip_reserved_program_name(programId: str): bool = all {
    sol.extracting,
    sol.pending.has(programId),
    not(sol.processed.contains(programId)),
    solEntryVerdict(sol, programId) == SolReservedName,
    sol' = solMarkRejected(sol, programId, "skip_reserved_program_name"),
    eth' = eth, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  action sol_accept_entry(programId: str): bool = all {
    sol.extracting,
    sol.pending.has(programId),
    not(sol.processed.contains(programId)),
    solEntryVerdict(sol, programId) == SolAccept,
    sol' = { ...sol,
      processed: sol.processed.union(Set(programId)),
      extracted: sol.extracted.put(programId, {
        body: sol.pending.get(programId).body,
        name: solResolvedName(sol.pending.get(programId)),
        signed: sol.pending.get(programId).signature != None }),
      notes: if (sol.pending.get(programId).signature == None)
               sol.notes.union(Set("accepted_unsigned")) else sol.notes },
    eth' = eth, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  /// `IdlRegistry::from_idl_mappings` -- the whole-map bulk writer. Its `Result` promises
  /// an error when any IDL JSON is invalid; it never returns one, and the parse failure
  /// it does hit (reading `metadata.name`) is discarded.
  action build_idl_registry: bool = all {
    not(sol.extracting),
    sol' = { ...sol, requestOk: true,
      registry: solBuildRegistry(sol.extracted),
      notes: if (sol.extracted.keys().exists(p => not(idlParseable(sol.extracted.get(p).body))))
               sol.notes.union(Set("registry_stored_unparseable_body")) else sol.notes },
    eth' = eth, prehashLedger' = prehashLedger, lastRead' = lastRead,
  }

  action read_has_idl(programId: str): bool = all {
    lastRead' =
      if (sol.registry.configs.has(programId))
        IdlAdvertised(sol.registry.configs.get(programId))
      else if (solHasBuiltinIdl(programId)) IdlAdvertised("builtin")
      else IdlNotAdvertised,
    eth' = eth, sol' = sol, prehashLedger' = prehashLedger,
  }

  action read_get_idl(programId: str): bool = all {
    lastRead' =
      if (not(sol.registry.configs.has(programId))) IdlAbsent
      else if (idlParseable(sol.registry.configs.get(programId)))
        IdlDecoded(sol.registry.configs.get(programId))
      else IdlUndecodable,
    eth' = eth, sol' = sol, prehashLedger' = prehashLedger,
  }

  action read_program_name(programId: str): bool = all {
    lastRead' = solProgramNameRead(sol.registry, programId),
    eth' = eth, sol' = sol, prehashLedger' = prehashLedger,
  }

  action read_idl_name(programId: str): bool = all {
    lastRead' = solIdlNameRead(sol.registry, programId),
    eth' = eth, sol' = sol, prehashLedger' = prehashLedger,
  }

  // ===========================================================================
  // step
  // ===========================================================================

  pure val BOOLS: Set[bool] = Set(false, true)
  /// `implementation_address` as supplied: a raw address string, or "" for the absent
  /// field. "" can never be a valid address, so it cannot collide with a real one.
  pure val NO_IMPL_ADDRESS: str = ""

  pure val ETH_IMPL_FIELD: Set[str] = ETH_MAP_KEYS.union(Set(NO_IMPL_ADDRESS))
  pure val KEY_SETS: Set[Set[str]] = Set(Set(), Set("curator"), Set("curator", "rogue"))
  pure val SIG_CAUSES: Set[str] = Set(
    "signature_rejected_missing_algorithm",
    "signature_rejected_unsupported_algorithm",
    "signature_rejected_missing_public_key",
    "signature_rejected_invalid_signature_encoding",
    "signature_rejected_invalid_public_key",
    "signature_rejected_verification_failed",
    "signature_rejected_not_in_allowlist",
  )

  /// Implementation-address shapes worth sampling evenly when an entry is declared a
  /// proxy: a resolvable target, no target at all, and a target that does not parse.
  pure val PROXY_TARGETS: Set[str] = Set(NO_IMPL_ADDRESS, ADDR_IMPL, ADDR_MALFORMED)

  // The arms are grouped so that the transitions a request actually walks through --
  // configure signers, offer an entry, sign it, run the loop, read the result -- stay at
  // top level, while the adversary variants and the direct registry API share arms. The
  // full contract of every action is unchanged; only how often the walk tries each one is.
  action step: bool = any {
    // ---- Ethereum env-driven signer configuration --------------------------
    all {
      not(eth.extracting),
      not(sol.extracting),
      any {
        nondet entries = ENV_LISTS.oneOf()
        set_eth_signer_env(entries),

        nondet devSigning = BOOLS.oneOf()
        build_eth_signer_allowlist(devSigning),
      },
    },

    // ---- Ethereum allowlist mutated directly -------------------------------
    all {
      not(eth.extracting),
      not(sol.extracting),
      any {
        nondet key = SIGNER_KEYS.oneOf()
        eth_allowlist_insert_key(key),

        nondet ks = KEY_SETS.oneOf()
        eth_allowlist_from_keys(ks),
      },
    },

    // ---- the caller supplies an ABI mapping entry -------------------------
    all {
      not(eth.extracting),
      not(sol.extracting),
      nondet mapKey = ETH_MAP_KEYS.oneOf()
      nondet body = ETH_BODIES.oneOf()
      offer_eth_abi_entry(mapKey, body),
    },

    // ---- the proxy pair: an implementation entry and a proxy pointing at it -
    all {
      not(eth.extracting),
      not(sol.extracting),
      any {
        all {
          not(eth.pending.has(ADDR_IMPL)),
          nondet body = Set("abiA", "abiB").oneOf()
          offer_eth_abi_entry(ADDR_IMPL, body),
        },
        all {
          eth.pending.has(ADDR_IMPL),
          not(eth.pending.has(ADDR_PROXY)),
          nondet body = Set("abiA", "abiB").oneOf()
          offer_eth_abi_entry(ADDR_PROXY, body),
        },
      },
    },

    // ---- link the proxy entry to the implementation entry beside it ---------
    // The one routing choice that lets proxy resolution reach an implementation ABI, so
    // it is sampled directly instead of needing the right key and target to coincide.
    all {
      not(eth.extracting),
      not(sol.extracting),
      eth.pending.has(ADDR_PROXY),
      eth.pending.has(ADDR_IMPL),
      set_eth_entry_routing(ADDR_PROXY, "proxy", ADDR_IMPL),
    },

    // ---- resolve a mapped proxy to its implementation ABI -------------------
    all {
      not(eth.extracting),
      not(sol.extracting),
      eth.registry.mappings.keys().exists(k =>
        eth.registry.mappings.get(k).abiKind == "proxy"),
      nondet slot = eth.registry.mappings.keys().filter(k =>
        eth.registry.mappings.get(k).abiKind == "proxy").oneOf()
      read_implementation_abi(slot._1, slot._2),
    },

    // ---- declare an entry a proxy, over the three target shapes -------------
    all {
      not(eth.extracting),
      not(sol.extracting),
      eth.pending.keys().size() > 0,
      nondet mapKey = eth.pending.keys().oneOf()
      nondet implKey = PROXY_TARGETS.oneOf()
      set_eth_entry_routing(mapKey, "proxy", implKey),
    },

    // ---- abi_type / implementation_address over their full domains ----------
    all {
      not(eth.extracting),
      not(sol.extracting),
      eth.pending.keys().size() > 0,
      nondet mapKey = eth.pending.keys().oneOf()
      nondet abiType = ABI_TYPES.oneOf()
      nondet implKey = ETH_IMPL_FIELD.oneOf()
      set_eth_entry_routing(mapKey, abiType, implKey),
    },

    // ---- the alias play: one address in two spellings, both admissible ------
    all {
      not(eth.extracting),
      not(sol.extracting),
      any {
        all {
          not(eth.pending.has(ADDR_TETHER)),
          nondet body = Set("abiA", "abiB").oneOf()
          offer_eth_abi_entry(ADDR_TETHER, body),
        },
        all {
          eth.pending.has(ADDR_TETHER),
          not(eth.pending.has(ADDR_TETHER_LOWER)),
          nondet body = Set("abiA", "abiB").oneOf()
          offer_eth_abi_entry(ADDR_TETHER_LOWER, body),
        },
      },
    },

    // ---- carry unread provenance, or drop the signature entirely ------------
    any {
      all {
        not(eth.extracting),
        eth.pending.keys().size() > 0,
        nondet mapKey = eth.pending.keys().oneOf()
        any { set_eth_signature_provenance(mapKey), strip_eth_signature(mapKey) },
      },
      all {
        not(sol.extracting),
        sol.pending.keys().size() > 0,
        nondet programId = sol.pending.keys().oneOf()
        any { set_idl_signature_provenance(programId), strip_idl_signature(programId) },
      },
    },

    // ---- the well-formed signature `sign_abi` produces ---------------------
    all {
      not(eth.extracting),
      not(sol.extracting),
      eth.pending.keys().size() > 0,
      eth.signers.keys.size() > 0,
      nondet mapKey = eth.pending.keys().oneOf()
      nondet signerKey = eth.signers.keys.oneOf()
      attach_eth_signature(mapKey, signerKey, mapKey, eth.chainId),
    },

    // ---- everything else that can happen to an entry before it is served ---
    all {
      not(eth.extracting),
      not(sol.extracting),
      eth.pending.keys().size() > 0,
      any {
        nondet mapKey = eth.pending.keys().oneOf()
        nondet signerKey = SIGNER_KEYS.union(Set(DEV_KEY)).oneOf()
        nondet boundKey = ETH_PARSEABLE_KEYS.oneOf()
        nondet boundChainId = ETH_CHAIN_IDS.oneOf()
        attach_eth_signature(mapKey, signerKey, boundKey, boundChainId),

        nondet mapKey = eth.pending.keys().oneOf()
        nondet defect = SIG_DEFECTS.oneOf()
        attach_defective_eth_signature(mapKey, defect),

        nondet mapKey = eth.pending.keys().oneOf()
        nondet firstKey = SIGNER_KEYS.union(Set(DEV_KEY)).oneOf()
        nondet shadowedKey = SIGNER_KEYS.union(Set(DEV_KEY)).oneOf()
        attach_eth_signature_with_shadowed_metadata(mapKey, firstKey, shadowedKey),

        nondet mapKey = eth.pending.keys().oneOf()
        nondet body = ETH_BODIES.oneOf()
        tamper_eth_abi_body(mapKey, body),
      },
    },

    // ---- the request is served --------------------------------------------
    all {
      not(sol.extracting),
      any {
        begin_eth_extraction(eth.chainId),

        nondet chainId = ETH_CHAIN_IDS.oneOf()
        begin_eth_extraction(chainId),
      },
    },

    // ---- the admission loop -----------------------------------------------
    // One arm per outcome, with the entry pick directly above the call: a pick nested
    // inside a further `any` is not visible to pin a replayed argument against.
    // Everything else in `step` is disabled while a request is in flight, so splitting
    // these costs nothing in simulation.
    all {
      eth.extracting,
      nondet mapKey = eth.pending.keys().oneOf()
      eth_skip_invalid_address(mapKey),
    },

    all {
      eth.extracting,
      nondet mapKey = eth.pending.keys().oneOf()
      eth_skip_oversized_body(mapKey),
    },

    all {
      eth.extracting,
      nondet mapKey = eth.pending.keys().oneOf()
      eth_skip_unparseable_abi_json(mapKey),
    },

    all {
      eth.extracting,
      nondet mapKey = eth.pending.keys().oneOf()
      nondet verdict = SIG_VERDICTS.oneOf()
      report_eth_signature_verdict(mapKey, verdict),
    },

    all {
      eth.extracting,
      nondet mapKey = eth.pending.keys().oneOf()
      nondet parses = BOOLS.oneOf()
      report_eth_abi_body_parse(mapKey, parses),
    },

    all {
      eth.extracting,
      nondet mapKey = eth.pending.keys().oneOf()
      nondet cause = SIG_CAUSES.oneOf()
      eth_skip_invalid_signature(mapKey, cause),
    },

    all {
      eth.extracting,
      nondet mapKey = eth.pending.keys().oneOf()
      eth_admit_entry(mapKey),
    },

    finish_eth_extraction,

    // ---- direct AbiRegistry API, registry reads, and the raw prehash --------
    all {
      not(eth.extracting),
      not(sol.extracting),
      any {
      all {
        not(eth.extracting),
        any {
          nondet name = ETH_ABI_NAMES.oneOf()
          nondet body = ETH_BODIES.oneOf()
          registry_register_abi(name, body),

          nondet chainId = ETH_CHAIN_IDS.oneOf()
          nondet addr = ETH_ADDRS.oneOf()
          nondet name = ETH_ABI_NAMES.oneOf()
          registry_map_address(chainId, addr, name),

          nondet chainId = ETH_CHAIN_IDS.oneOf()
          nondet addr = ETH_ADDRS.oneOf()
          nondet name = ETH_ABI_NAMES.oneOf()
          nondet kind = Set("implementation", "proxy").oneOf()
          nondet impl = Set(None, Some(ADDR_IMPL), Some(ADDR_TETHER)).oneOf()
          registry_map_address_with_type(chainId, addr, name, kind, impl),
        },
      },

      nondet probe = PREHASH_PROBES.oneOf()
      compute_metadata_prehash(probe),

      nondet name = ETH_MAP_KEYS.union(ETH_ABI_NAMES).oneOf()
      read_abi_by_name(name),

      nondet chainId = ETH_CHAIN_IDS.oneOf()
      read_registry_listing(chainId),
      },
    },

    // ---- Ethereum registry reads -------------------------------------------
    all {
      not(eth.extracting),
      not(sol.extracting),
      any {
      // Guided: read a slot the registry actually holds, so the resolved outcomes do not
      // depend on the walk guessing a mapped (chain id, address) pair.
      all {
        eth.registry.mappings.keys().size() > 0,
        nondet slot = eth.registry.mappings.keys().oneOf()
        any {
          read_abi_for_address(slot._1, slot._2),
          read_abi_kind(slot._1, slot._2),
          read_implementation_abi(slot._1, slot._2),
        },
      },

      // Unguided: any address, so the unmapped outcomes stay reachable too.
      nondet chainId = ETH_CHAIN_IDS.oneOf()
      nondet addr = ETH_ADDRS.oneOf()
      any {
        read_abi_for_address(chainId, addr),
        read_abi_kind(chainId, addr),
        read_implementation_abi(chainId, addr),
      },
      },
    },

    // ---- Solana env-driven signer configuration (the OnceLock path) ---------
    all {
      not(sol.extracting),
      not(eth.extracting),
      any {
        nondet entries = ENV_LISTS.oneOf()
        set_sol_signer_env(entries),

        build_sol_signer_allowlist,
      },
    },

    // ---- Solana allowlist mutated directly ----------------------------------
    all {
      not(sol.extracting),
      not(eth.extracting),
      any {
        nondet key = SIGNER_KEYS.oneOf()
        sol_allowlist_insert_key(key),

        nondet ks = KEY_SETS.oneOf()
        sol_allowlist_from_keys(ks),
      },
    },

    // ---- the caller supplies an IDL mapping entry --------------------------
    all {
      not(sol.extracting),
      not(eth.extracting),
      nondet programId = SOL_PROGRAM_KEYS.oneOf()
      nondet body = SOL_BODIES.oneOf()
      offer_idl_entry(programId, body),
    },

    // ---- a caller IDL for a program that has no canonical identity ----------
    all {
      not(sol.extracting),
      not(eth.extracting),
      nondet programId = SOL_OVERRIDABLE_PROGRAMS.oneOf()
      nondet body = Set("idlA", "idlBadJson").oneOf()
      offer_idl_entry(programId, body),
    },

    // ---- the impersonation play's opening move: a body whose `metadata.name` is a
    //      reserved canonical label, offered against a pubkey with no canonical identity
    all {
      not(sol.extracting),
      not(eth.extracting),
      nondet programId = SOL_OVERRIDABLE_PROGRAMS.oneOf()
      offer_idl_entry(programId, "idlSpoofName"),
    },

    // ---- the well-formed curator signature ---------------------------------
    all {
      not(sol.extracting),
      not(eth.extracting),
      sol.pending.keys().size() > 0,
      sol.signers.keys.size() > 0,
      nondet programId = sol.pending.keys().oneOf()
      nondet signerKey = sol.signers.keys.oneOf()
      attach_idl_signature(programId, signerKey, programId),
    },

    // ---- the impersonation play the reserved-name guard is meant to stop ----
    all {
      not(sol.extracting),
      not(eth.extracting),
      sol.pending.keys().exists(p =>
        RESERVED_CANONICAL_NAMES.contains(
          solMetaNameOr(sol.pending.get(p).body, SOL_FALLBACK_NAME))),
      nondet programId = sol.pending.keys().filter(p =>
        RESERVED_CANONICAL_NAMES.contains(
          solMetaNameOr(sol.pending.get(p).body, SOL_FALLBACK_NAME))).oneOf()
      set_idl_program_name(programId, "My Wallet"),
    },

    // ---- everything else that can happen to an IDL entry -------------------
    all {
      not(sol.extracting),
      not(eth.extracting),
      sol.pending.keys().size() > 0,
      any {
        nondet programId = sol.pending.keys().oneOf()
        nondet name = SOL_PROGRAM_NAMES.oneOf()
        set_idl_program_name(programId, name),

        nondet programId = sol.pending.keys().oneOf()
        nondet signerKey = SIGNER_KEYS.oneOf()
        nondet boundProgramId = SOL_OVERRIDABLE_PROGRAMS.oneOf()
        attach_idl_signature(programId, signerKey, boundProgramId),

        nondet programId = sol.pending.keys().oneOf()
        nondet defect = SIG_DEFECTS.oneOf()
        attach_defective_idl_signature(programId, defect),

        nondet programId = sol.pending.keys().oneOf()
        nondet firstKey = SIGNER_KEYS.oneOf()
        nondet shadowedKey = SIGNER_KEYS.oneOf()
        attach_idl_signature_with_shadowed_metadata(programId, firstKey, shadowedKey),

        nondet programId = sol.pending.keys().oneOf()
        nondet body = SOL_BODIES.oneOf()
        tamper_idl_body(programId, body),
      },
    },

    // ---- the Solana request is served --------------------------------------
    all { not(eth.extracting), begin_sol_extraction },

    all {
      sol.extracting,
      nondet programId = sol.pending.keys().oneOf()
      sol_skip_invalid_program_id(programId),
    },

    all {
      sol.extracting,
      nondet programId = sol.pending.keys().oneOf()
      sol_skip_trusted_program(programId),
    },

    all {
      sol.extracting,
      nondet programId = sol.pending.keys().oneOf()
      sol_skip_oversized_body(programId),
    },

    all {
      sol.extracting,
      nondet programId = sol.pending.keys().oneOf()
      sol_skip_reserved_program_name(programId),
    },

    all {
      sol.extracting,
      nondet programId = sol.pending.keys().oneOf()
      nondet verdict = SIG_VERDICTS.oneOf()
      report_idl_signature_verdict(programId, verdict),
    },

    all {
      not(sol.extracting),
      not(eth.extracting),
      sol.pending.keys().size() > 0,
      nondet programId = sol.pending.keys().oneOf()
      nondet oversized = BOOLS.oneOf()
      report_idl_body_size(programId, oversized),
    },

    finish_sol_extraction,

    all {
      sol.extracting,
      nondet programId = sol.pending.keys().oneOf()
      nondet cause = SIG_CAUSES.oneOf()
      sol_skip_invalid_signature(programId, cause),
    },

    all {
      sol.extracting,
      nondet programId = sol.pending.keys().oneOf()
      sol_accept_entry(programId),
    },

    build_idl_registry,

    // ---- Solana reads against a program the registry actually holds ---------
    all {
      not(eth.extracting),
      not(sol.extracting),
      sol.registry.configs.keys().size() > 0,
      nondet programId = sol.registry.configs.keys().oneOf()
      any {
        read_has_idl(programId),
        read_get_idl(programId),
        read_program_name(programId),
        read_idl_name(programId),
      },
    },

    // ---- Solana reads against any program id --------------------------------
    all {
      not(eth.extracting),
      not(sol.extracting),
      any {
      nondet programId = SOL_PROGRAM_KEYS.oneOf()
      any {
        read_has_idl(programId),
        read_get_idl(programId),
        read_program_name(programId),
        read_idl_name(programId),
      },
      },
    },
  }

  // ===========================================================================
  // Observations -- one per outcome class the code distinguishes
  // ===========================================================================

  /// A caller ABI was admitted and decodes its own address's calldata directly.
  val eth_entry_admitted_as_implementation: bool =
    eth.registry.mappings.keys().exists(k => {
      val m = eth.registry.mappings.get(k)
      m.installedByMetadataKey != None and m.abiKind == "implementation"
    })

  /// A caller ABI was admitted as a proxy WITH a resolvable implementation link.
  val eth_proxy_admitted_with_implementation_link: bool =
    eth.registry.mappings.keys().exists(k => {
      val m = eth.registry.mappings.get(k)
      m.installedByMetadataKey != None and m.abiKind == "proxy" and m.implementation != None
    })

  /// A proxy entry whose `implementation_address` was missing or unparseable was kept as
  /// a proxy with no link rather than dropped.
  val eth_proxy_admitted_without_implementation_link: bool =
    eth.registry.mappings.keys().exists(k => {
      val m = eth.registry.mappings.get(k)
      m.installedByMetadataKey != None and m.abiKind == "proxy" and m.implementation == None
    })

  /// The proxy's `implementation_address` was present but did not parse -- a distinct log
  /// site from a proxy that simply omitted the field.
  val eth_proxy_implementation_address_was_malformed: bool =
    eth.notes.contains("proxy_implementation_address_malformed")

  /// An entry carrying NO signature was registered anyway (graceful degradation).
  val eth_unsigned_entry_admitted: bool = eth.notes.contains("accepted_unsigned")

  /// An entry whose curator signature verified AND whose signer was allowlisted.
  val eth_curator_signed_entry_admitted: bool =
    eth.registry.mappings.keys().exists(k =>
      eth.registry.mappings.get(k).vouchedBody != None)

  val eth_entry_skipped_for_invalid_address: bool =
    eth.notes.contains("skip_invalid_address")

  val eth_entry_skipped_for_oversized_body: bool =
    eth.notes.contains("skip_oversized_body")

  /// Signature and allowlist both passed, then `serde_json::from_str::<JsonAbi>` failed:
  /// the ABI body never enters the registry.
  val eth_entry_skipped_for_unparseable_abi_json: bool =
    eth.notes.contains("skip_unparseable_abi_json")

  /// The signature verified but the key that made it is not an authorized curator.
  val eth_signature_rejected_for_unauthorized_signer: bool =
    eth.notes.contains("signature_rejected_not_in_allowlist")

  /// The signature did not verify over the recomputed prehash -- the body was changed
  /// after signing, or the signature was minted for a different chain id or address.
  val eth_signature_rejected_for_wrong_preimage: bool =
    eth.notes.contains("signature_rejected_verification_failed")

  /// `algorithm` was missing, or was something other than secp256k1.
  val eth_signature_rejected_for_unsupported_algorithm: bool =
    eth.notes.contains("signature_rejected_missing_algorithm")
      or eth.notes.contains("signature_rejected_unsupported_algorithm")

  /// The signature or public-key bytes did not decode at all.
  val eth_signature_rejected_for_malformed_key_material: bool =
    eth.notes.contains("signature_rejected_missing_public_key")
      or eth.notes.contains("signature_rejected_invalid_signature_encoding")
      or eth.notes.contains("signature_rejected_invalid_public_key")

  /// Every offered entry was rejected, so `try_extract_from_chain_metadata` returns
  /// `None` and the request proceeds with no caller ABIs at all.
  val eth_extraction_yielded_no_registry: bool =
    eth.notes.contains("extraction_returned_no_registry")

  /// Two spellings of one address were both registered as ABIs while only one address
  /// mapping exists -- the displaced body stays visible to `list_abis` but unreachable.
  val eth_alias_keys_collapsed_to_one_mapping: bool =
    eth.registry.abis.has(ADDR_TETHER) and eth.registry.abis.has(ADDR_TETHER_LOWER)
      and ETH_CHAIN_IDS.exists(c => eth.registry.mappings.has((c, ADDR_TETHER)))

  /// A mapping names an ABI that was never registered -- `map_address_with_type` does not
  /// check, so `get_abi_for_address` returns `None` for a mapped address.
  val eth_mapping_points_at_unregistered_abi: bool =
    eth.registry.mappings.keys().exists(k =>
      not(eth.registry.abis.has(eth.registry.mappings.get(k).abiName)))

  /// An admitted entry carries `issuer`/`timestamp` that the prehash does not cover and
  /// no validation step reads.
  val eth_admitted_entry_carries_unverified_provenance: bool =
    eth.admittedKeys.exists(k =>
      eth.pending.has(k) and
      match eth.pending.get(k).signature {
        | None => false
        | Some(s) => unverifiedFieldsPresent(s).size() > 0
      })

  /// Calldata at a proxy was decoded against the LINKED implementation's ABI.
  val eth_implementation_resolved_through_proxy_link: bool =
    match lastRead { | EthImplResolved(_) => true | _ => false }

  /// Proxy resolution failed and the rendering fell back to the proxy's own ABI.
  val eth_calldata_fell_back_to_proxy_own_abi: bool =
    match lastRead { | EthImplUnresolved(_) => true | _ => false }

  /// A proto declared `algorithm` or `public_key` twice and the verifier silently took the
  /// first of the two rather than rejecting the entry as ambiguous.
  val signature_metadata_duplicate_resolved_by_list_order: bool =
    eth.notes.contains("signature_metadata_duplicate_resolved_by_list_order")
      or sol.notes.contains("signature_metadata_duplicate_resolved_by_list_order")

  /// Every configured curator key was dropped as malformed, leaving an allowlist that
  /// authorizes nothing while the operator believes a curator is configured.
  val eth_allowlist_built_empty_after_dropping_every_key: bool =
    eth.signers.built and eth.signers.dropped > 0 and eth.signers.keys.size() == 0

  val idl_entry_accepted_unsigned: bool = sol.notes.contains("accepted_unsigned")

  val idl_entry_accepted_curator_signed: bool =
    sol.extracted.keys().exists(p => sol.extracted.get(p).signed)

  val idl_skipped_for_invalid_program_id: bool =
    sol.notes.contains("skip_invalid_program_id")

  /// A caller tried to override a native, core-SPL, built-in-IDL or preset-registered
  /// program and the body was refused.
  val idl_skipped_for_trusted_program: bool = sol.notes.contains("skip_trusted_program")

  val idl_skipped_for_oversized_body: bool = sol.notes.contains("skip_oversized_body")

  val idl_skipped_for_invalid_signature: bool =
    SIG_CAUSES.exists(c => sol.notes.contains(c))

  /// The resolved display name was a canonical label belonging to a different program.
  val idl_skipped_for_reserved_program_name: bool =
    sol.notes.contains("skip_reserved_program_name")

  /// `has_idl` says a decoder exists for a program whose stored body cannot be decoded.
  val idl_advertised_but_undecodable: bool =
    sol.registry.configs.keys().exists(p => not(idlParseable(sol.registry.configs.get(p))))

  /// A reserved canonical label reached `idl_names` on a pubkey with no canonical
  /// identity, because the guard only ever tested the resolved `program_name`.
  val idl_metadata_name_bypassed_the_reserved_guard: bool =
    sol.registry.idlNames.keys().exists(p =>
      solCanonicalName(p) == None
        and RESERVED_CANONICAL_NAMES.contains(sol.registry.idlNames.get(p)))

  /// A trusted program's canonical name was returned instead of anything caller-supplied.
  val canonical_program_name_returned: bool =
    match lastRead { | ProgramNameCanonical(_) => true | _ => false }

  /// A caller-supplied name was rendered for a program with no canonical identity.
  val caller_supplied_program_name_returned: bool =
    match lastRead { | ProgramNameCaller(_) => true | _ => false }

  /// A caller `metadata.name` was suppressed because the program has a canonical identity.
  val caller_idl_name_suppressed_for_canonical_program: bool =
    lastRead == IdlNameSuppressed

  /// The cached, process-lifetime IDL signer allowlist ended up empty even though the
  /// deployment configured signer entries.
  val sol_allowlist_cached_empty_for_process_life: bool =
    sol.signers.built and sol.signers.env.length() > 0 and sol.signers.keys.size() == 0

  // ===========================================================================
  // Correctness properties
  // ===========================================================================

  // Without the le_u64 length prefix, bytes shift across the scope/body boundary and one
  // signature becomes valid for a different contract or program.
  /// Distinct (chain_tag, scope, body) triples never share a prehash preimage.
  val prehash_encoding_is_injective: bool =
    prehashLedger.forall(a => prehashLedger.forall(b =>
      (a.digest == b.digest) implies (a.input == b.input)))

  // A present-but-invalid signature signals tampering; treating it as merely absent would
  // let a tampered body decide what the human reads.
  /// An entry whose present signature failed validation is never registered.
  val present_but_invalid_signature_is_never_admitted: bool = and {
    eth.rejectedKeys.intersect(eth.registry.abis.keys()) == Set(),
    sol.rejectedKeys.intersect(sol.registry.configs.keys()) == Set(),
  }

  // An oversized body would otherwise be carried into the per-request hot path.
  /// Caller metadata above the per-request size bound never reaches the registry.
  val oversized_caller_metadata_never_reaches_the_registry: bool = and {
    eth.admittedBodies.forall(b => not(abiOversized(b))),
    sol.registry.configs.keys().forall(p => not(idlOversized(sol.registry.configs.get(p)))),
  }

  // An unparseable body would make the registry advertise a decoder for an address it
  // cannot actually decode.
  /// Every ABI body the registry holds parses as a JSON ABI.
  val registered_abi_body_is_always_parseable: bool =
    eth.registry.abis.keys().forall(n => abiParseable(eth.registry.abis.get(n)))

  // An attacker body would otherwise drive argument and account naming for a program the
  // signer recognises by name.
  /// No caller-supplied IDL body is ever stored for a trusted built-in or preset program.
  val trusted_program_never_gets_a_caller_supplied_idl: bool =
    sol.registry.configs.keys().forall(p => not(solIsTrusted(p)))

  // Otherwise a compromised wallet could relabel the System Program in the signing payload.
  /// A program with a canonical name always renders under that canonical name.
  val canonical_program_name_is_never_caller_controlled: bool =
    SOL_PROGRAM_KEYS.forall(p =>
      match solCanonicalName(p) {
        | Some(cn) => solProgramNameRead(sol.registry, p) == ProgramNameCanonical(cn)
        | None => true
      })

  // The rendered "Program (name: ...)" label must not be influenced by untrusted metadata
  // when the program has a canonical identity.
  /// A caller `metadata.name` is never shown for a program with a canonical name.
  val canonical_program_never_shows_a_caller_idl_name: bool =
    SOL_PROGRAM_KEYS.forall(p =>
      (solCanonicalName(p) != None)
        implies (solIdlNameRead(sol.registry, p) == IdlNameSuppressed))

  // One rejected entry must only remove itself from the mapping it arrived in.
  /// Every entry the admission loop admitted is still registered after later entries run.
  val one_bad_entry_never_drops_a_good_one: bool =
    eth.admittedKeys.forall(k => eth.registry.abis.has(k))

  // A mapping to an unregistered name makes `get_abi_for_address` return `None` for an
  // address the registry reports as mapped, so calldata there silently renders as raw hex.
  /// Every address mapping names an ABI the registry actually holds.
  val every_mapping_resolves_to_a_registered_abi: bool =
    eth.registry.mappings.keys().forall(k =>
      eth.registry.abis.has(eth.registry.mappings.get(k).abiName))

  // Otherwise a caller that signs its ABI correctly is dropped while the same caller that
  // omits the signature has its decoder shown to the human.
  /// An allowlist that authorizes nobody admits no caller-supplied ABI.
  val empty_allowlist_admits_no_caller_abi: bool =
    (eth.signers.built and eth.signers.keys.size() == 0) implies
      eth.registry.mappings.keys().forall(k =>
        eth.registry.mappings.get(k).installedByMetadataKey == None)

  // Otherwise an attacker who cannot forge a curator signature simply omits one, and the
  // allowlist constrains only honest callers.
  /// Once a curator allowlist is configured, only curator-signed IDLs enter the registry.
  val configured_curator_allowlist_requires_signed_idls: bool =
    (sol.signers.keys.size() > 0) implies
      sol.registry.configs.keys().forall(p =>
        sol.extracted.has(p) and sol.extracted.get(p).signed)

  // `abi_type` and `implementation_address` sit outside the signed preimage, so flipping a
  // signed implementation to a proxy pointing elsewhere leaves the signature valid.
  /// An address a curator vouched for is decoded against exactly the body that curator signed.
  val curator_vouched_address_decodes_the_vouched_body: bool =
    eth.registry.mappings.keys().forall(k =>
      match eth.registry.mappings.get(k).vouchedBody {
        | None => true
        | Some(b) => ethDecoderBody(eth.registry, k._1, k._2) == Some(b)
      })

  // One typo in a deployment env var would otherwise invert the trust model in silence.
  /// Dropping malformed curator keys never leaves signed ABIs as the only rejected ones.
  val dropped_signer_keys_never_silently_invert_admission: bool =
    not(and {
      eth.signers.dropped > 0,
      eth.signers.keys.size() == 0,
      eth.notes.contains("signature_rejected_not_in_allowlist"),
      eth.notes.contains("accepted_unsigned"),
    })

  // Otherwise a caller appends a case-variant duplicate of a curated address and its own
  // body decides what the human reads, while the displaced body still reports as registered.
  /// A caller-supplied mapping is never replaced by a second map key spelling the same address.
  val admitted_metadata_mapping_is_never_shadowed: bool =
    eth.registry.mappings.keys().forall(slot =>
      match eth.registry.mappings.get(slot).installedByMetadataKey {
        | None => true
        | Some(k) => eth.registry.firstMetadataInstaller.getOrElse(slot, k) == k
      })

  // `issuer` and `timestamp` look like provenance and freshness but bind nothing, and with
  // no expiry check a signature minted once is admitted forever.
  /// An admitted entry's signature carries no field outside both the prehash and the verifier's reads.
  val admitted_signature_carries_no_unverified_fields: bool = and {
    eth.admittedKeys.forall(k =>
      not(eth.pending.has(k)) or
      match eth.pending.get(k).signature {
        | None => true
        | Some(s) => unverifiedFieldsPresent(s).size() == 0
      }),
    sol.extracted.keys().forall(p =>
      not(sol.pending.has(p)) or
      match sol.pending.get(p).signature {
        | None => true
        | Some(s) => unverifiedFieldsPresent(s).size() == 0
      }),
  }

  // Otherwise the whole curator path is off for the life of the process and only a warning
  // line records it.
  /// A deployment whose configured curator keys all dropped never answers a request as Ok.
  val disarmed_curator_config_is_surfaced_to_the_caller: bool =
    (sol.signers.built and sol.signers.env.length() > 0 and sol.signers.keys.size() == 0)
      implies not(sol.requestOk)

  // The `OnceLock` builds the allowlist once per process, so a configuration corrected
  // after the first parse request never takes effect.
  /// The cached IDL signer allowlist matches the signer configuration currently set.
  val sol_allowlist_tracks_the_signer_env: bool =
    sol.signers.built implies (sol.signers.builtFromEnv == sol.signers.env)

  // `has_idl` and `get_idl` would otherwise disagree, and the unparseable body is stored
  // with the built-in-override flag set.
  /// The registry never advertises an IDL whose stored body cannot be parsed.
  val advertised_idl_is_always_decodable: bool =
    sol.registry.configs.keys().forall(p => idlParseable(sol.registry.configs.get(p)))

  // The compile-time `CLI_DEV_SIGNING_KEY_SEED` is an all-0x42 seed whose public key is
  // allowlisted whenever `dev-signing` or `cfg(test)` is on, so a shipped binary built with
  // that feature accepts ABI signatures anyone can mint.
  /// Every key an admitted ABI was accepted on came from the deployment's signer configuration.
  val curator_key_admitting_an_abi_was_deployment_configured: bool =
    eth.registry.mappings.keys().forall(slot =>
      match eth.registry.mappings.get(slot).vouchedSigner {
        | None => true
        | Some(signer) => envKeys(eth.signers.env).contains(signer)
      })

  // The proto metadata list is scanned with `.find`, so a duplicate `algorithm` or
  // `public_key` pair is resolved by list order and the shadowed value is never reported.
  /// An entry whose signature metadata declares a key twice is never admitted.
  val ambiguous_signature_metadata_is_never_admitted: bool = and {
    eth.ambiguousKeys.intersect(eth.admittedKeys) == Set(),
    sol.ambiguousKeys.intersect(sol.extracted.keys()) == Set(),
  }

  // Both loops `continue` past every rejected entry, so a caller supplying a hundred
  // tampered mappings still gets a rendering built from whichever few survived.
  /// A caller mapping with any dropped entry never produces a rendering as if none dropped.
  val partial_caller_mapping_never_silently_renders: bool = and {
    eth.returnedRegistry implies eth.rejectedKeys.size() == 0,
    sol.requestOk implies sol.rejectedKeys.size() == 0,
  }

  // Otherwise a benign `program_name` walks past the impersonation guard while the body's
  // `metadata.name` puts a trusted program's label into the signing payload.
  /// A reserved canonical program label never renders on a pubkey with no canonical identity.
  val reserved_canonical_label_never_renders_on_a_stranger: bool =
    sol.registry.idlNames.keys().forall(p =>
      (solCanonicalName(p) == None) implies
        not(RESERVED_CANONICAL_NAMES.contains(sol.registry.idlNames.get(p))))

  // ===========================================================================
  // Scenario runs
  // ===========================================================================

  /// A proxy whose `implementation_address` cannot be parsed is kept as a proxy with no
  /// link, and calldata at it renders against the proxy's own ABI instead of a blank
  /// raw-hex screen.
  run proxyWithMalformedTargetStillDecodesTest =
    init
      .then(set_eth_signer_env(List("curator")))
      .then(build_eth_signer_allowlist(false))
      .then(offer_eth_abi_entry(ADDR_PROXY, "abiA"))
      .then(set_eth_entry_routing(ADDR_PROXY, "proxy", ADDR_MALFORMED))
      .then(attach_eth_signature(ADDR_PROXY, "curator", ADDR_PROXY, 1))
      .then(begin_eth_extraction(1))
      .then(eth_admit_entry(ADDR_PROXY))
      .then(finish_eth_extraction)
      .expect(and {
        eth.returnedRegistry,
        eth.registry.mappings.get((1, ADDR_PROXY)).abiKind == "proxy",
        eth.registry.mappings.get((1, ADDR_PROXY)).implementation == None,
        eth.notes.contains("proxy_implementation_address_malformed"),
      })
      .then(read_implementation_abi(1, ADDR_PROXY))
      .expect(lastRead == EthImplUnresolved("no_link"))
      .then(read_abi_for_address(1, ADDR_PROXY))
      .expect(and {
        lastRead == EthAbiFound("abiA"),
        ethDecoderBody(eth.registry, 1, ADDR_PROXY) == Some("abiA"),
      })
}
