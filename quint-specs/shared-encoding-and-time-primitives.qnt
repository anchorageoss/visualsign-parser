// -*- mode: quint -*-
//
// Shared Encoding And Time Primitives -- the bottom-layer helpers every chain
// crate is required to route through:
//
//   src/visualsign/src/encodings.rs
//     strip_hex_prefix, split_hex_prefix, decode_hex,
//     SupportedEncodings::{detect, as_str, Display, FromStr}
//   src/visualsign/src/time_fmt.rs
//     format_timestamp_ms, format_relative_ms
//
// These are TOTAL PURE FUNCTIONS -- there is no product state and no operation
// has a precondition, so this spec has NO guards. It is modelled the same way
// `deterministic-serialization` is: as a CALL LEDGER. Every action performs one
// real call and appends a record holding the arguments that went in beside the
// result that came out; the correctness properties then relate those records to
// each other, or to an INDEPENDENT recomputation from the recorded arguments.
// That is what makes them falsifiable: a wrong prefix rule, a wrong threshold or
// a dropped range check shows up as a disagreement between two records that the
// contract says must agree.
//
// HOW STRINGS ARE MODELLED
//   Quint has no string operations, so a hex/base64 input is a `List[str]` of
//   single-character strings. That is enough to express everything these helpers
//   actually inspect: the two leading characters (the `0x`/`0X` prefix), whether
//   each remaining character is an ASCII hex digit, the body's length parity, and
//   the nibble value of each digit. The input domain is built as
//   BODIES x {none, "0x", "0X"} so it is CLOSED UNDER PREFIXING -- without that
//   closure the headline prefix-invariance property would have no pair to relate.
//
// HOW TIME IS MODELLED
//   `format_timestamp_ms` renders with `%Y-%m-%d %H:%M:%S`, i.e. SECOND
//   granularity, so its output is modelled as `FormattedUtc(floor(ms/1000))` --
//   injective in the rendered second, and deliberately NOT injective in `ms`,
//   which is the sub-second collapse the callers work around by printing the raw
//   epoch alongside. `format_relative_ms` is modelled structurally
//   (`Span({n, unit, future, approx, plural})`) because the wording is a total
//   function of those five values, and the properties are about them.
//
// WHAT THIS COVERS
//   * every public operation of encodings.rs and time_fmt.rs
//   * prefix acceptance: optional (strip/decode) vs mandatory (split), single
//     strip only, and the residual-prefix consequence
//   * the hex decoder's three outcome classes (bytes, odd length, invalid char)
//     and its two silent quirks (empty succeeds, digit case is ignored)
//   * format detection, including the case where a base64 payload is ALSO
//     well-formed hex and is therefore routed to the hex decoder
//   * both timestamp helpers' representable/unrepresentable arms and every arm of
//     the relative unit ladder, in both directions
//
// WHAT THIS DOES NOT COVER
//   * chrono's calendar arithmetic: representability is a numeric range check
//     against chrono's DateTime<Utc> bounds, and the rendered date is represented
//     by the second it falls in rather than by digits
//   * base64 DECODING: `SupportedEncodings` only classifies; the base64 decode
//     itself lives in the chain crates. "Would also decode as base64" is modelled
//     conservatively (standard alphabet, length a multiple of four, no padding),
//     which can only UNDER-report ambiguity, never invent it
//   * `hex::FromHexError`'s Display text; the error is modelled as its variant
//     plus, for an invalid character, the index the crate reports
//   * the i128/u128 WIDENING inside `format_relative_ms`: a span that overflows an
//     i64 subtraction needs a `now_ms` beyond +/-9.2e18, and the simulator's own
//     integer arithmetic is i64, so such a span cannot be evaluated here. The
//     reachable half of that hazard IS modelled: `now_ms` is never validated, so
//     the domain includes reference points the parser itself cannot render, and
//     the wording stays confident relative to them
//   * the per-chain callers that consume these results (their own components)
//
// CONFORMANCE
//   Every action also writes a FLAT projection of its result into the `facts`
//   ghost variable, and the instrumented Rust pins that field with an
//   `assert!(facts / <op>, ..)` derived from the value the real function actually
//   returned. Since none of these operations has a precondition, that pin is what
//   makes a replayed unit test check anything at all: it forces this spec's
//   `decodeHex`, `stripHexPrefix`, `detectEncoding`, `formatTimestampMs`,
//   `formatRelativeMs` (and friends) to agree with the Rust on every value the
//   suite exercises. The projections are deliberately primitive-only (bools, ints,
//   strings, lists) so both sides compute the same shape independently -- the Rust
//   side derives the relative-wording fact by PARSING the string the function
//   returned, never by recomputing the ladder.
module shared_encoding_and_time_primitives {

  // ===========================================================================
  // 0. Characters and strings
  // ===========================================================================

  /// A string, as the only thing these helpers can see about it: its characters,
  /// in order. Single-character `str`s keep traces readable and keep the eventual
  /// instrumentation trivial (`s.chars().map(String::from).collect()`).
  type Str = List[str]

  /// `char::is_ascii_hexdigit` -- note `x` and `X` are NOT in this set, which is
  /// exactly why `detect` can strip a prefix and still test "all hex digits".
  pure val HEX_DIGITS: Set[str] = Set(
    "0", "1", "2", "3", "4", "5", "6", "7", "8", "9",
    "a", "b", "c", "d", "e", "f",
    "A", "B", "C", "D", "E", "F",
  )

  pure val NIBBLE: str -> int = Map(
    "0" -> 0, "1" -> 1, "2" -> 2, "3" -> 3, "4" -> 4,
    "5" -> 5, "6" -> 6, "7" -> 7, "8" -> 8, "9" -> 9,
    "a" -> 10, "b" -> 11, "c" -> 12, "d" -> 13, "e" -> 14, "f" -> 15,
    "A" -> 10, "B" -> 11, "C" -> 12, "D" -> 13, "E" -> 14, "F" -> 15,
  )

  /// `str::to_ascii_lowercase`, restricted to the characters this domain uses.
  pure val TO_LOWER: str -> str = Map(
    "A" -> "a", "B" -> "b", "C" -> "c", "D" -> "d", "E" -> "e", "F" -> "f",
    "X" -> "x",
  )

  /// The standard base64 alphabet (RFC 4648), padding excluded.
  pure val B64_ALPHABET: Set[str] = Set(
    "0", "1", "2", "3", "4", "5", "6", "7", "8", "9",
    "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m",
    "n", "o", "p", "q", "r", "s", "t", "u", "v", "w", "x", "y", "z",
    "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M",
    "N", "O", "P", "Q", "R", "S", "T", "U", "V", "W", "X", "Y", "Z",
    "+", "/",
  )

  pure def isHexDigit(c: str): bool = HEX_DIGITS.contains(c)

  /// Only ever applied to a character the caller has already established is a hex
  /// digit (`bytesOf` runs after the all-hex-digits check, exactly as the `hex`
  /// crate does).
  pure def nibble(c: str): int = NIBBLE.get(c)

  pure def lowerChar(c: str): str =
    if (TO_LOWER.keys().contains(c)) TO_LOWER.get(c) else c

  pure def lowerStr(s: Str): Str =
    s.foldl(List(), (acc, c) => acc.append(lowerChar(c)))

  /// Every character is an ASCII hex digit -- the `chars().all(..)` test in
  /// `SupportedEncodings::detect` (encodings.rs:17).
  pure def allHexDigits(s: Str): bool =
    s.indices().forall(i => isHexDigit(s.nth(i)))

  /// Index of the first non-hex-digit character, or -1. This is the `hex` crate's
  /// own scan (it reports `InvalidHexCharacter { c, index }`), kept SEPARATE from
  /// `allHexDigits` on purpose: `detect` and `decode_hex` are two independent
  /// implementations of "is this hex", and one of the properties below is exactly
  /// that they agree.
  pure def firstNonHexIndex(s: Str): int =
    s.foldl({ i: 0, found: -1 }, (a, c) =>
      if (a.found >= 0) { ...a, i: a.i + 1 }
      else if (isHexDigit(c)) { ...a, i: a.i + 1 }
      else { i: a.i + 1, found: a.i }).found

  /// Pair the digits up, high nibble first. Only called on an even-length,
  /// all-hex-digit body.
  pure def bytesOf(s: Str): List[int] =
    s.foldl({ hi: -1, out: List() }, (a, c) =>
      if (a.hi < 0) { ...a, hi: nibble(c) }
      else { hi: -1, out: a.out.append(a.hi * 16 + nibble(c)) }).out

  // ===========================================================================
  // 1. Prefix handling -- encodings.rs:65, :76
  // ===========================================================================

  /// Which prefix a caller put in front of a body. `prefixed` is the CONSTRUCTOR
  /// side of the prefix contract; `stripHexPrefix` below is the destructor side.
  /// Keeping the two independent is what makes prefix-invariance falsifiable.
  type Prefix = NoPrefix | PrefixLower | PrefixUpper

  pure def prefixed(p: Prefix, body: Str): Str =
    match p {
      | NoPrefix    => body
      | PrefixLower => List("0", "x").concat(body)
      | PrefixUpper => List("0", "X").concat(body)
    }

  /// Which of the two accepted prefix spellings a value carries, if any. Written
  /// as a cascade rather than a conjunction so `nth` is never reached on a string
  /// too short to have those positions.
  type PrefixKind = KindNone | KindLower | KindUpper

  pure def prefixKindOf(s: Str): PrefixKind =
    if (s.length() < 2) KindNone
    else if (s.nth(0) != "0") KindNone
    else if (s.nth(1) == "x") KindLower
    else if (s.nth(1) == "X") KindUpper
    else KindNone

  pure def carriesPrefix(s: Str): bool = prefixKindOf(s) != KindNone

  /// `strip_hex_prefix` (encodings.rs:65): `strip_prefix("0x").or_else(|| strip_prefix("0X"))`,
  /// falling back to the input unchanged. Exactly ONE prefix is removed and the
  /// digits are never validated.
  pure def stripHexPrefix(value: Str): Str =
    if (carriesPrefix(value)) value.slice(2, value.length()) else value

  /// `split_hex_prefix` (encodings.rs:76): the mandatory-prefix variant used where
  /// the `0x` is required (JSON-RPC quantities, `validate_eth_address`).
  type MaybeBody = FoundBody(Str) | NoPrefixFound

  pure def splitHexPrefix(value: Str): MaybeBody =
    if (carriesPrefix(value)) FoundBody(value.slice(2, value.length())) else NoPrefixFound

  // ===========================================================================
  // 2. Hex decoding -- encodings.rs:88, via the `hex` crate
  // ===========================================================================

  /// `hex::FromHexError`, restricted to the two variants `hex::decode` can return.
  /// The crate checks LENGTH FIRST, so an odd-length body with a bad character is
  /// reported as `OddLength`, never as an invalid character.
  type HexError = OddLength | InvalidHexCharacter({ index: int })

  type DecodeResult = Decoded(List[int]) | DecodeFailed(HexError)

  /// `decode_hex` (encodings.rs:88) = `hex::decode(strip_hex_prefix(value))`.
  pure def decodeHex(value: Str): DecodeResult = {
    val body = stripHexPrefix(value)
    if (body.length() % 2 != 0) DecodeFailed(OddLength)
    else if (firstNonHexIndex(body) >= 0)
      DecodeFailed(InvalidHexCharacter({ index: firstNonHexIndex(body) }))
    else Decoded(bytesOf(body))
  }

  pure def decodeSucceeded(r: DecodeResult): bool =
    match r {
      | Decoded(_)     => true
      | DecodeFailed(_) => false
    }

  // ===========================================================================
  // 3. Format detection and encoding names -- encodings.rs:16, :28, :45
  // ===========================================================================

  type SupportedEncodings = Hex | Base64

  /// `SupportedEncodings::detect` (encodings.rs:16). Total: it never fails and
  /// never reports "unknown" -- everything that is not all-hex-digits after the
  /// prefix is assumed to be base64.
  pure def detectEncoding(data: Str): SupportedEncodings =
    if (allHexDigits(stripHexPrefix(data))) Hex else Base64

  /// `as_str` / `Display` (encodings.rs:28, :35): the canonical lowercase names
  /// that appear in CLI output and in `FromStr`'s error text.
  pure def encodingAsStr(e: SupportedEncodings): str =
    match e {
      | Base64 => "base64"
      | Hex    => "hex"
    }

  type EncodingParse = ParsedEncoding(SupportedEncodings) | UnsupportedEncodingName

  /// `str::to_lowercase` over the names a caller can plausibly pass. Anything not
  /// listed is already lowercase (or contains no cased letters).
  pure val LOWERCASE_NAME: str -> str = Map(
    "HEX" -> "hex", "Hex" -> "hex", "hEx" -> "hex",
    "BASE64" -> "base64", "Base64" -> "base64", "baseSixtyFour" -> "basesixtyfour",
  )

  pure def lowercaseName(s: str): str =
    if (LOWERCASE_NAME.keys().contains(s)) LOWERCASE_NAME.get(s) else s

  /// `impl FromStr for SupportedEncodings` (encodings.rs:45): lowercase, then match
  /// the two canonical names; anything else is an error naming both formats.
  pure def encodingFromStr(s: str): EncodingParse = {
    val lower = lowercaseName(s)
    if (lower == "base64") ParsedEncoding(Base64)
    else if (lower == "hex") ParsedEncoding(Hex)
    else UnsupportedEncodingName
  }

  /// Conservative "this would also decode as standard base64": alphabet-only,
  /// non-empty, length a multiple of four, no padding. Under-reports ambiguity
  /// (a padded payload is not counted) and never over-reports it.
  pure def wellFormedBase64(s: Str): bool =
    and {
      s.length() > 0,
      s.length() % 4 == 0,
      s.indices().forall(i => B64_ALPHABET.contains(s.nth(i))),
    }

  // ===========================================================================
  // 4. Timestamps -- time_fmt.rs:14, :25
  // ===========================================================================

  /// The bounds of `chrono::DateTime<Utc>`: -262144-01-01T00:00:00Z through
  /// +262143-12-31T23:59:59.999Z, in epoch milliseconds. `from_timestamp_millis`
  /// returns `None` outside them -- and both i64 extremes are outside.
  pure val CHRONO_MIN_MS: int = -8334632937600000
  pure val CHRONO_MAX_MS: int = 8210298412799999
  pure val I64_MAX: int = 9223372036854775807
  /// Written as `-(i64::MAX) - 1`: the literal itself is outside the range the
  /// evaluator accepts, the value is not.
  pure val I64_MIN: int = (0 - I64_MAX) - 1

  pure def representableMs(ms: int): bool =
    ms >= CHRONO_MIN_MS and ms <= CHRONO_MAX_MS

  /// Floor division by 1000 -- the calendar second `ms` falls in. Written out
  /// rather than using `/`, because `/` truncates toward zero and a pre-epoch
  /// timestamp must round DOWN to match chrono.
  pure def secondOf(ms: int): int =
    if (ms >= 0) ms / 1000 else 0 - (((0 - ms) + 999) / 1000)

  /// The result of `format_timestamp_ms`. `FormattedUtc` carries the SECOND, not
  /// the millisecond: the format string is `%Y-%m-%d %H:%M:%S`, so two epochs in
  /// the same second render as the same string.
  type TimestampText = FormattedUtc(int) | InvalidTimestampMarker

  /// `format_timestamp_ms` (time_fmt.rs:14).
  pure def formatTimestampMs(ms: int): TimestampText =
    if (representableMs(ms)) FormattedUtc(secondOf(ms)) else InvalidTimestampMarker

  type Unit = Second | Minute | Hour | Day | Month

  pure val SEC: int = 1000
  pure val MIN_MS: int = 60 * 1000
  pure val HOUR_MS: int = 60 * 60 * 1000
  pure val DAY_MS: int = 24 * 60 * 60 * 1000
  /// A FLAT 30 days -- `format_relative_ms` has no calendar awareness.
  pure val MONTH_MS: int = 30 * 24 * 60 * 60 * 1000

  pure def unitMs(u: Unit): int =
    match u {
      | Second => SEC
      | Minute => MIN_MS
      | Hour   => HOUR_MS
      | Day    => DAY_MS
      | Month  => MONTH_MS
    }

  /// The wording `format_relative_ms` produces, held as the values that determine
  /// it: `"in about 2 hours"` is `Span({n: 2, unit: Hour, future: true,
  /// approx: true, plural: true})`.
  type Relative = JustNow | Span({ n: int, unit: Unit, future: bool, approx: bool, plural: bool })

  type RelativeText = Rendered(Relative) | Omitted

  /// `format_relative_ms` (time_fmt.rs:25). Note `now_ms` is NOT validated -- only
  /// `ms` goes through chrono -- and the difference is widened to i128/u128 so a
  /// MIN-vs-MAX span cannot overflow.
  pure def formatRelativeMs(ms: int, now_ms: int): RelativeText =
    if (not(representableMs(ms))) Omitted
    else {
      val diff = ms - now_ms
      val absMs = if (diff < 0) 0 - diff else diff
      if (absMs < SEC) Rendered(JustNow)
      else {
        val pick =
          if (absMs < MIN_MS)       { n: absMs / SEC,      unit: Second, approx: false }
          else if (absMs < HOUR_MS) { n: absMs / MIN_MS,   unit: Minute, approx: false }
          else if (absMs < DAY_MS)  { n: absMs / HOUR_MS,  unit: Hour,   approx: true }
          else if (absMs < MONTH_MS){ n: absMs / DAY_MS,   unit: Day,    approx: true }
          else                      { n: absMs / MONTH_MS, unit: Month,  approx: true }
        Rendered(Span({
          n: pick.n,
          unit: pick.unit,
          future: diff > 0,
          approx: pick.approx,
          plural: pick.n != 1,
        }))
      }
    }

  pure def unitName(u: Unit): str =
    match u {
      | Second => "second"
      | Minute => "minute"
      | Hour   => "hour"
      | Day    => "day"
      | Month  => "month"
    }

  // ===========================================================================
  // 4b. Conformance facts -- the flat projection each call pins
  // ===========================================================================

  type StripFact      = { output: Str, removedPrefix: bool }
  type SplitFact      = { found: bool, body: Str }
  /// `badIndex` is -1 when no character was at fault, so an invalid-character
  /// error at index 0 stays distinguishable from "no error position".
  type DecodeFact     = { ok: bool, bytes: List[int], oddLength: bool, badIndex: int }
  type DetectFact     = { isHex: bool }
  type NameParseFact  = { ok: bool, name: str }
  type NameRenderFact = { name: str }
  /// `second` is the calendar second the rendered date names, 0 when invalid.
  type TimestampFact  = { valid: bool, second: int }
  type RelativeFact   = { rendered: bool, justNow: bool, n: int, unit: str,
                          future: bool, approx: bool, plural: bool }

  pure def stripFact(value: Str): StripFact =
    { output: stripHexPrefix(value), removedPrefix: carriesPrefix(value) }

  pure def splitFact(value: Str): SplitFact =
    match splitHexPrefix(value) {
      | FoundBody(b)   => { found: true, body: b }
      | NoPrefixFound  => { found: false, body: List() }
    }

  pure def decodeFact(r: DecodeResult): DecodeFact =
    match r {
      | Decoded(bs)     => { ok: true, bytes: bs, oddLength: false, badIndex: -1 }
      | DecodeFailed(e) => match e {
          | OddLength                => { ok: false, bytes: List(), oddLength: true, badIndex: -1 }
          | InvalidHexCharacter(pos) => { ok: false, bytes: List(), oddLength: false,
                                          badIndex: pos.index }
        }
    }

  pure def detectFact(e: SupportedEncodings): DetectFact = { isHex: e == Hex }

  pure def nameParseFact(r: EncodingParse): NameParseFact =
    match r {
      | ParsedEncoding(e)        => { ok: true, name: encodingAsStr(e) }
      | UnsupportedEncodingName  => { ok: false, name: "" }
    }

  pure def timestampFact(t: TimestampText): TimestampFact =
    match t {
      | FormattedUtc(sec)      => { valid: true, second: sec }
      | InvalidTimestampMarker => { valid: false, second: 0 }
    }

  /// The projection the Rust side reproduces by PARSING the returned wording
  /// ("in about 2 hours" -> n 2, unit "hour", future, approx, plural).
  pure def relativeFact(t: RelativeText): RelativeFact =
    match t {
      | Omitted     => { rendered: false, justNow: false, n: 0, unit: "",
                         future: false, approx: false, plural: false }
      | Rendered(v) => match v {
          | JustNow => { rendered: true, justNow: true, n: 0, unit: "",
                         future: false, approx: false, plural: false }
          | Span(p) => { rendered: true, justNow: false, n: p.n, unit: unitName(p.unit),
                         future: p.future, approx: p.approx, plural: p.plural }
        }
    }

  /// One field per operation, each holding the projection of that operation's most
  /// recent call. The instrumented code pins the field its call site owns; the
  /// other fields carry over untouched.
  type Facts = {
    strip: StripFact,
    split: SplitFact,
    decode: DecodeFact,
    detect: DetectFact,
    nameParse: NameParseFact,
    nameRender: NameRenderFact,
    timestamp: TimestampFact,
    relative: RelativeFact,
  }

  pure val NO_FACTS: Facts = {
    strip: { output: List(), removedPrefix: false },
    split: { found: false, body: List() },
    decode: { ok: false, bytes: List(), oddLength: false, badIndex: -1 },
    detect: { isHex: false },
    nameParse: { ok: false, name: "" },
    nameRender: { name: "" },
    timestamp: { valid: false, second: 0 },
    relative: { rendered: false, justNow: false, n: 0, unit: "",
                future: false, approx: false, plural: false },
  }

  // ===========================================================================
  // 5. The call ledger
  // ===========================================================================

  type StripRecord      = { input: Str, output: Str }
  type SplitRecord      = { input: Str, output: MaybeBody }
  type DecodeRecord     = { input: Str, result: DecodeResult }
  type DetectRecord     = { input: Str, result: SupportedEncodings }
  type NameParseRecord  = { input: str, result: EncodingParse }
  type NameRenderRecord = { input: SupportedEncodings, output: str }
  type TimestampRecord  = { ms: int, text: TimestampText }
  type RelativeRecord   = { ms: int, now: int, text: RelativeText }

  /// Every call the system made, oldest first, each recording its arguments beside
  /// its result. The history is state because almost every guarantee here RELATES
  /// two calls (prefixed vs unprefixed, upper- vs lowercase, absolute vs relative)
  /// rather than constraining any single one.
  type Ledger = {
    strips: List[StripRecord],
    splits: List[SplitRecord],
    decodes: List[DecodeRecord],
    detects: List[DetectRecord],
    nameParses: List[NameParseRecord],
    nameRenders: List[NameRenderRecord],
    timestamps: List[TimestampRecord],
    relatives: List[RelativeRecord],
  }

  pure val EMPTY_LEDGER: Ledger = {
    strips: List(),
    splits: List(),
    decodes: List(),
    detects: List(),
    nameParses: List(),
    nameRenders: List(),
    timestamps: List(),
    relatives: List(),
  }

  var ledger: Ledger

  /// The conformance projection of the most recent call to each operation. Ghost
  /// state: no property reads it -- the instrumented code pins it at replay.
  var facts: Facts

  // ===========================================================================
  // 6. The input domains
  // ===========================================================================

  /// The bodies the inline tests and the chain call sites actually exercise.
  /// Every one is combined with all three prefix forms below, so the domain is
  /// closed under prefixing.
  pure val BODY_EMPTY: Str      = List()
  pure val BODY_ABCD: Str       = List("a", "b", "c", "d")
  pure val BODY_ABCD_MIXED: Str = List("a", "b", "C", "D")
  pure val BODY_ABCD_UPPER: Str = List("A", "B", "C", "D")
  pure val BODY_ODD: Str        = List("a", "b", "c")
  pure val BODY_ONE_DIGIT: Str  = List("1")
  pure val BODY_ZEROS: Str      = List("0", "0")
  pure val BODY_NON_HEX: Str    = List("z", "z")
  /// `strip_hex_prefix("0x0Xab") == "0Xab"` -- the residual-prefix test.
  pure val BODY_LOWER_PREFIXED: Str = List("0", "x", "a", "b")
  pure val BODY_UPPER_PREFIXED: Str = List("0", "X", "a", "b")
  /// Eight hex digits: valid hex AND a well-formed base64 word -- the ambiguity.
  pure val BODY_DEADBEEF: Str   = List("d", "e", "a", "d", "b", "e", "e", "f")
  /// `detect_treats_prefixed_hex_as_hex`'s negative case.
  pure val BODY_NOT_HEX: Str    = List("n", "o", "t", "-", "h", "e", "x", "+", "/", "=")

  pure val BODIES: Set[Str] = Set(
    BODY_EMPTY, BODY_ABCD, BODY_ABCD_MIXED, BODY_ABCD_UPPER, BODY_ODD,
    BODY_ONE_DIGIT, BODY_ZEROS, BODY_NON_HEX, BODY_LOWER_PREFIXED,
    BODY_UPPER_PREFIXED, BODY_DEADBEEF, BODY_NOT_HEX,
  )

  pure val HEX_INPUTS: Set[Str] =
    BODIES
      .union(BODIES.map(b => prefixed(PrefixLower, b)))
      .union(BODIES.map(b => prefixed(PrefixUpper, b)))

  /// `--encoding` / config values a caller can pass to `FromStr`.
  pure val ENCODING_NAMES: Set[str] = Set(
    "hex", "HEX", "Hex", "hEx",
    "base64", "BASE64", "Base64",
    "", "utf8", "baseSixtyFour",
  )

  pure val ENCODINGS: Set[SupportedEncodings] = Set(Hex, Base64)

  /// 2023-11-14 22:13:20 UTC -- the `NOW` the time_fmt tests are written against.
  pure val NOW: int = 1700000000000

  pure val TIMES: Set[int] = Set(
    NOW,
    // sub-second, both directions: these render as the SAME string as their
    // neighbours in the same calendar second, which is the collapse the callers
    // work around by printing the raw epoch alongside.
    NOW + 1, NOW + 500, NOW + 999,
    NOW - 1, NOW - 500, NOW - 999,
    NOW + SEC, NOW - 45 * SEC,                  // seconds
    NOW + MIN_MS, NOW - 2 * MIN_MS,             // minutes
    NOW + HOUR_MS, NOW - 23 * HOUR_MS,          // hours
    NOW + DAY_MS, NOW - 5 * DAY_MS,             // days
    NOW + 2 * MONTH_MS, NOW - 6 * MONTH_MS,     // months
    NOW + 31 * DAY_MS,                          // the flat-30-day month boundary
    NOW - (MONTH_MS - 1),                       // one ms short of a "month"
    0,
    CHRONO_MAX_MS, CHRONO_MAX_MS + 1,
    CHRONO_MIN_MS, CHRONO_MIN_MS - 1,
    I64_MAX, I64_MIN,
  )

  /// Including reference points the parser itself cannot render: only `ms` goes
  /// through chrono, so a nonsense `now_ms` is accepted silently.
  pure val NOWS: Set[int] = Set(NOW, 0, CHRONO_MAX_MS + 1, CHRONO_MIN_MS - 1)

  // ===========================================================================
  // 7. Actions -- one real call each
  // ===========================================================================

  action init: bool = all {
    ledger' = EMPTY_LEDGER,
    facts' = NO_FACTS,
  }

  /// `visualsign::encodings::strip_hex_prefix(value)` (encodings.rs:65).
  /// Call sites include `normalize_eth_address` (ethereum/src/cli_plugin.rs:85).
  action strip_hex_prefix(value: Str): bool = all {
    ledger' = { ...ledger, strips: ledger.strips.append({
      input: value,
      output: stripHexPrefix(value),
    }) },
    facts' = { ...facts, strip: stripFact(value) },
  }

  /// `visualsign::encodings::split_hex_prefix(value)` (encodings.rs:76).
  /// Call sites: `require_hex_prefix` (ethereum/src/eth_json.rs:227) and
  /// `validate_eth_address` (ethereum/src/cli_plugin.rs:66).
  action split_hex_prefix(value: Str): bool = all {
    ledger' = { ...ledger, splits: ledger.splits.append({
      input: value,
      output: splitHexPrefix(value),
    }) },
    facts' = { ...facts, split: splitFact(value) },
  }

  /// `visualsign::encodings::decode_hex(value)` (encodings.rs:88). Call sites:
  /// ethereum/src/lib.rs:432, solana core/visualsign.rs:86,
  /// sui core/transaction/decoder.rs:27.
  action decode_hex(value: Str): bool = all {
    ledger' = { ...ledger, decodes: ledger.decodes.append({
      input: value,
      result: decodeHex(value),
    }) },
    facts' = { ...facts, decode: decodeFact(decodeHex(value)) },
  }

  /// `SupportedEncodings::detect(data)` (encodings.rs:16). Call sites:
  /// ethereum/src/lib.rs:150, solana core/visualsign.rs:80,
  /// sui core/visualsign.rs:46.
  action detect_encoding(data: Str): bool = all {
    ledger' = { ...ledger, detects: ledger.detects.append({
      input: data,
      result: detectEncoding(data),
    }) },
    facts' = { ...facts, detect: detectFact(detectEncoding(data)) },
  }

  /// `SupportedEncodings::from_str(s)` (encodings.rs:45).
  action encoding_from_str(s: str): bool = all {
    ledger' = { ...ledger, nameParses: ledger.nameParses.append({
      input: s,
      result: encodingFromStr(s),
    }) },
    facts' = { ...facts, nameParse: nameParseFact(encodingFromStr(s)) },
  }

  /// `SupportedEncodings::as_str` / `Display::fmt` (encodings.rs:28, :35) -- one
  /// action, because `Display` is implemented as `write!(f, "{}", self.as_str())`.
  action encoding_as_str(encoding: SupportedEncodings): bool = all {
    ledger' = { ...ledger, nameRenders: ledger.nameRenders.append({
      input: encoding,
      output: encodingAsStr(encoding),
    }) },
    facts' = { ...facts, nameRender: { name: encodingAsStr(encoding) } },
  }

  /// `format_timestamp_ms(ms)` (time_fmt.rs:14). Call site: tron/src/lib.rs:559.
  action format_timestamp_ms(ms: int): bool = all {
    ledger' = { ...ledger, timestamps: ledger.timestamps.append({
      ms: ms,
      text: formatTimestampMs(ms),
    }) },
    facts' = { ...facts, timestamp: timestampFact(formatTimestampMs(ms)) },
  }

  /// `format_relative_ms(ms, now_ms)` (time_fmt.rs:25). Call site:
  /// tron/src/lib.rs:560.
  action format_relative_ms(ms: int, now_ms: int): bool = all {
    ledger' = { ...ledger, relatives: ledger.relatives.append({
      ms: ms,
      now: now_ms,
      text: formatRelativeMs(ms, now_ms),
    }) },
    facts' = { ...facts, relative: relativeFact(formatRelativeMs(ms, now_ms)) },
  }

  action step: bool = {
    nondet value = HEX_INPUTS.oneOf()
    nondet data = HEX_INPUTS.oneOf()
    nondet s = ENCODING_NAMES.oneOf()
    nondet encoding = ENCODINGS.oneOf()
    nondet ms = TIMES.oneOf()
    nondet now_ms = NOWS.oneOf()
    any {
      strip_hex_prefix(value),
      split_hex_prefix(value),
      decode_hex(value),
      detect_encoding(data),
      encoding_from_str(s),
      encoding_as_str(encoding),
      format_timestamp_ms(ms),
      format_relative_ms(ms, now_ms),
    }
  }

  // ===========================================================================
  // 8. Observations -- the outcome classes worth measuring coverage for
  // ===========================================================================

  // -- strip_hex_prefix ------------------------------------------------------

  /// A lowercase `0x` prefix was consumed.
  val strip_removed_a_lowercase_prefix: bool =
    ledger.strips.foldl(false, (seen, r) =>
      seen or (prefixKindOf(r.input) == KindLower and r.output != r.input))

  /// An uppercase `0X` prefix was consumed -- the `or_else` arm.
  val strip_removed_an_uppercase_prefix: bool =
    ledger.strips.foldl(false, (seen, r) =>
      seen or (prefixKindOf(r.input) == KindUpper and r.output != r.input))

  /// The `unwrap_or(value)` arm: no prefix, the input came back untouched.
  val strip_passed_through_unprefixed: bool =
    ledger.strips.foldl(false, (seen, r) => seen or r.output == r.input)

  /// Only ONE prefix was stripped, so a doubly-prefixed value still carries one.
  val strip_left_a_residual_prefix: bool =
    ledger.strips.foldl(false, (seen, r) => seen or carriesPrefix(r.output))

  // -- split_hex_prefix ------------------------------------------------------

  /// The mandatory-prefix variant accepted a value and handed back the body.
  val split_accepted_a_prefixed_value: bool =
    ledger.splits.foldl(false, (seen, r) => seen or r.output != NoPrefixFound)

  /// The mandatory-prefix variant refused an unprefixed value -- the case that
  /// becomes "must start with 0x" in `validate_eth_address`.
  val split_rejected_an_unprefixed_value: bool =
    ledger.splits.foldl(false, (seen, r) => seen or r.output == NoPrefixFound)

  // -- decode_hex ------------------------------------------------------------

  /// A prefixed value decoded to bytes.
  val decoded_with_prefix: bool =
    ledger.decodes.foldl(false, (seen, r) =>
      seen or (carriesPrefix(r.input) and decodeSucceeded(r.result)))

  /// A bare (unprefixed) value decoded to bytes.
  val decoded_without_prefix: bool =
    ledger.decodes.foldl(false, (seen, r) =>
      seen or (not(carriesPrefix(r.input)) and decodeSucceeded(r.result)))

  /// The silent case-insensitivity: uppercase `A`-`F` digits decoded fine.
  pure val UPPER_HEX_LETTERS: Set[str] = Set("A", "B", "C", "D", "E", "F")

  val decoded_uppercase_hex_digits: bool =
    ledger.decodes.foldl(false, (seen, r) => {
      val body = stripHexPrefix(r.input)
      seen or (decodeSucceeded(r.result)
                 and body.indices().exists(i => UPPER_HEX_LETTERS.contains(body.nth(i))))
    })

  /// The QUIRK: `""` and `"0x"` decode to zero bytes rather than erroring, so a
  /// caller that relies on a decode error to reject empty input gets an empty
  /// payload instead.
  val decoded_empty_to_zero_bytes: bool =
    ledger.decodes.foldl(false, (seen, r) => seen or r.result == Decoded(List()))

  /// The `hex` crate's length check fired.
  val decode_failed_odd_length: bool =
    ledger.decodes.foldl(false, (seen, r) => seen or r.result == DecodeFailed(OddLength))

  /// The `hex` crate's per-character check fired.
  val decode_failed_invalid_character: bool =
    ledger.decodes.foldl(false, (seen, r) =>
      match r.result {
        | DecodeFailed(e) => seen or e != OddLength
        | Decoded(_)      => seen
      })

  // -- detect ----------------------------------------------------------------

  /// A prefixed value was classified Hex -- only possible because `detect` strips
  /// the prefix before testing, since `x` is not an ASCII hex digit.
  val detected_hex_after_stripping_prefix: bool =
    ledger.detects.foldl(false, (seen, r) =>
      seen or (carriesPrefix(r.input) and r.result == Hex))

  /// A bare all-hex-digit value was classified Hex.
  val detected_bare_hex: bool =
    ledger.detects.foldl(false, (seen, r) =>
      seen or (not(carriesPrefix(r.input)) and r.result == Hex))

  /// The `else` arm: something that is not all hex digits was assumed to be base64.
  val detected_base64: bool =
    ledger.detects.foldl(false, (seen, r) => seen or r.result == Base64)

  /// THE ADVERSARY CLASS: a payload that is well-formed base64 AND all hex digits
  /// was routed to the hex decoder. There is no fallback at any call site
  /// (ethereum lib.rs:150, solana visualsign.rs:80, sui visualsign.rs:46), so the
  /// bytes the signer sees come from the wrong decoder.
  val detected_ambiguous_payload_as_hex: bool =
    ledger.detects.foldl(false, (seen, r) => or {
      seen,
      and { r.result == Hex, wellFormedBase64(r.input), decodeSucceeded(decodeHex(r.input)) },
    })

  // -- encoding names --------------------------------------------------------

  /// A canonical lowercase name parsed.
  val encoding_name_parsed_canonical: bool =
    ledger.nameParses.foldl(false, (seen, r) => or {
      seen,
      and { r.result != UnsupportedEncodingName, lowercaseName(r.input) == r.input },
    })

  /// A name that needed `to_lowercase` first parsed -- the case-insensitivity arm.
  val encoding_name_parsed_mixed_case: bool =
    ledger.nameParses.foldl(false, (seen, r) => or {
      seen,
      and { r.result != UnsupportedEncodingName, lowercaseName(r.input) != r.input },
    })

  /// The `_ =>` arm: an unknown name produced the "Supported formats are" error.
  val encoding_name_rejected: bool =
    ledger.nameParses.foldl(false, (seen, r) => seen or r.result == UnsupportedEncodingName)

  /// `as_str`'s `Hex` arm.
  val encoding_rendered_as_hex_name: bool =
    ledger.nameRenders.foldl(false, (seen, r) => seen or r.input == Hex)

  /// `as_str`'s `Base64` arm.
  val encoding_rendered_as_base64_name: bool =
    ledger.nameRenders.foldl(false, (seen, r) => seen or r.input == Base64)

  // -- timestamps ------------------------------------------------------------

  /// A representable epoch rendered as a UTC date.
  val timestamp_rendered_utc: bool =
    ledger.timestamps.foldl(false, (seen, r) => seen or r.text != InvalidTimestampMarker)

  /// The `unwrap_or_else` arm: the literal "invalid timestamp".
  val timestamp_reported_unrepresentable: bool =
    ledger.timestamps.foldl(false, (seen, r) => seen or r.text == InvalidTimestampMarker)

  /// The SECOND-granularity quirk: two different epochs rendered as the same
  /// string, which is why callers print the raw `ms` alongside.
  val timestamp_collapsed_sub_second: bool =
    ledger.timestamps.indices().exists(i => ledger.timestamps.indices().exists(j => {
      val a = ledger.timestamps.nth(i)
      val b = ledger.timestamps.nth(j)
      and { a.ms != b.ms, a.text != InvalidTimestampMarker, a.text == b.text }
    }))

  // -- relative wording ------------------------------------------------------

  val relative_just_now: bool =
    ledger.relatives.foldl(false, (seen, r) => seen or r.text == Rendered(JustNow))

  pure def spanUnitIs(t: RelativeText, u: Unit): bool =
    match t {
      | Omitted     => false
      | Rendered(v) => match v {
          | JustNow => false
          | Span(p) => p.unit == u
        }
    }

  pure def spanIsFuture(t: RelativeText): bool =
    match t {
      | Omitted     => false
      | Rendered(v) => match v {
          | JustNow => false
          | Span(p) => p.future
        }
    }

  pure def spanIsPast(t: RelativeText): bool =
    match t {
      | Omitted     => false
      | Rendered(v) => match v {
          | JustNow => false
          | Span(p) => not(p.future)
        }
    }

  val relative_in_seconds: bool =
    ledger.relatives.foldl(false, (seen, r) => seen or spanUnitIs(r.text, Second))

  val relative_in_minutes: bool =
    ledger.relatives.foldl(false, (seen, r) => seen or spanUnitIs(r.text, Minute))

  val relative_in_hours: bool =
    ledger.relatives.foldl(false, (seen, r) => seen or spanUnitIs(r.text, Hour))

  val relative_in_days: bool =
    ledger.relatives.foldl(false, (seen, r) => seen or spanUnitIs(r.text, Day))

  val relative_in_months: bool =
    ledger.relatives.foldl(false, (seen, r) => seen or spanUnitIs(r.text, Month))

  /// The `"in ..."` branch.
  val relative_future_direction: bool =
    ledger.relatives.foldl(false, (seen, r) => seen or spanIsFuture(r.text))

  /// The `"... ago"` branch.
  val relative_past_direction: bool =
    ledger.relatives.foldl(false, (seen, r) => seen or spanIsPast(r.text))

  /// The `?` on `DateTime::from_timestamp_millis` fired: the caller is told to omit
  /// the relative tag rather than being handed a misleading one.
  val relative_omitted_for_unrepresentable: bool =
    ledger.relatives.foldl(false, (seen, r) => seen or r.text == Omitted)

  /// `now_ms` was a reference point the parser cannot itself render, and the
  /// relative tag was still emitted: only `ms` is validated, so the wording is
  /// confident about a span measured from nonsense.
  val relative_measured_against_unrepresentable_now: bool =
    ledger.relatives.foldl(false, (seen, r) =>
      seen or (r.text != Omitted and not(representableMs(r.now))))

  /// The FLAT-30-DAY quirk: a span of 31 or more days was rendered as "about N
  /// month(s)" with N computed from 30-day months, so a signer reading a relative
  /// expiry is short of the real calendar date.
  val relative_month_rounded_from_flat_30_days: bool =
    ledger.relatives.foldl(false, (seen, r) => or {
      seen,
      and {
        spanUnitIs(r.text, Month),
        (if (r.ms - r.now < 0) r.now - r.ms else r.ms - r.now) >= 31 * DAY_MS,
      },
    })

  // ===========================================================================
  // 9. Correctness properties
  // ===========================================================================

  // Independently computed: the `prefixed` CONSTRUCTOR on one side, the
  // `stripHexPrefix` destructor on the other. A `strip_prefix("0x")` that forgot
  // its `or_else("0X")` arm falsifies it, and the same transaction would then
  // render differently depending on whether the wallet sent it prefixed.
  /// Adding a `0x` or `0X` prefix to an unprefixed value never changes the bytes `decode_hex` returns.
  val prefix_never_changes_decoded_bytes: bool =
    ledger.decodes.indices().forall(i => ledger.decodes.indices().forall(j => {
      val a = ledger.decodes.nth(i)
      val b = ledger.decodes.nth(j)
      (and {
        splitHexPrefix(b.input) == NoPrefixFound,
        a.input == prefixed(PrefixLower, b.input) or a.input == prefixed(PrefixUpper, b.input),
      }) implies a.result == b.result
    }))

  // If the two diverged, an address missing its `0x` would pass
  // `validate_eth_address` and be normalized into a well-formed-looking address
  // that is not the one the user typed.
  /// `split_hex_prefix` finds no prefix in exactly the values `strip_hex_prefix` returns unchanged.
  val mandatory_prefix_rejects_exactly_the_passthroughs: bool =
    ledger.strips.foldl(true, (ok, r) =>
      ok and ((splitHexPrefix(r.input) == NoPrefixFound) iff (r.output == r.input)))

  // Exactly one prefix is stripped. A loop-stripping implementation would accept
  // `"0x0x41"` as the byte `0x41`, letting a caller smuggle a value past a
  // prefix-shape check.
  /// A value carrying two `0x` prefixes always fails to decode.
  val residual_prefix_never_decodes: bool =
    ledger.decodes.foldl(true, (ok, r) =>
      ok and ((carriesPrefix(r.input) and carriesPrefix(stripHexPrefix(r.input)))
                implies not(decodeSucceeded(r.result))))

  // If `detect` skipped the prefix strip, `"0xab..."` would be classified base64
  // and handed to the base64 decoder, and every prefixed transaction would fail
  // to parse.
  /// Every value `decode_hex` accepts is classified as `Hex` by `detect`.
  val every_decodable_input_is_detected_as_hex: bool =
    ledger.detects.foldl(true, (ok, r) =>
      ok and (decodeSucceeded(decodeHex(r.input)) implies r.result == Hex))

  // The fixed-size-array call sites (ethereum `abi_metadata.rs:302`, solana
  // `idl/signature.rs:113`) check only the LENGTH of the result, so a decoder that
  // dropped or duplicated a nibble would hand them a signature or pubkey built
  // from shifted bytes.
  /// A successful decode returns exactly half as many bytes as the body has characters.
  val decoded_length_is_half_the_body: bool =
    ledger.decodes.foldl(true, (ok, r) =>
      match r.result {
        | Decoded(bs)     => ok and bs.length() * 2 == stripHexPrefix(r.input).length()
        | DecodeFailed(_) => ok
      })

  // A checksummed (EIP-55 mixed-case) address must produce the same bytes as its
  // lowercase form, or one account would be shown as two different addresses
  // depending on how the wallet cased it.
  /// Two values that differ only in ASCII letter case decode to the same result.
  val letter_case_never_changes_decoded_bytes: bool =
    ledger.decodes.indices().forall(i => ledger.decodes.indices().forall(j => {
      val a = ledger.decodes.nth(i)
      val b = ledger.decodes.nth(j)
      (lowerStr(a.input) == lowerStr(b.input)) implies a.result == b.result
    }))

  // If `format_relative_ms` dropped its `DateTime::from_timestamp_millis(ms)?`
  // guard, a signer would be shown "invalid timestamp (in about 292471208 years)"
  // -- confident wording attached to a date the parser admits it cannot represent.
  /// `format_relative_ms` omits the relative tag for exactly the epochs `format_timestamp_ms` reports as invalid.
  val absolute_and_relative_agree_on_representability: bool =
    ledger.relatives.foldl(true, (ok, r) =>
      ok and ((r.text == Omitted) iff (formatTimestampMs(r.ms) == InvalidTimestampMarker)))

  // A sign flip here tells a signer that an expiry already passed when it is still
  // in the future (or the reverse), which is the difference between refusing and
  // approving a transaction.
  /// Relative wording reads as future exactly when `ms` is after `now_ms`, is plural exactly when the count is not one, and never reports zero units.
  val relative_wording_is_direction_and_number_consistent: bool =
    ledger.relatives.foldl(true, (ok, r) =>
      match r.text {
        | Omitted     => ok
        | Rendered(v) => match v {
            | JustNow => ok
            | Span(p) => and {
                ok,
                p.future == (r.ms > r.now),
                p.plural == (p.n != 1),
                p.n >= 1,
              }
          }
      })

  // Computed by multiplication, independently of the code's division, so an
  // off-by-one in the unit ladder -- rendering a 23-hour gap as "about 1 hour" --
  // is caught. Months are the code's flat 30 days.
  /// The reported count of units never exceeds the real span and one more unit always does, and "just now" is used only below one second.
  val relative_magnitude_brackets_the_real_span: bool =
    ledger.relatives.foldl(true, (ok, r) =>
      match r.text {
        | Omitted     => ok
        | Rendered(v) => {
            val absMs = if (r.ms - r.now < 0) r.now - r.ms else r.ms - r.now
            match v {
              | JustNow => ok and absMs < SEC
              | Span(p) => and {
                  ok,
                  absMs >= SEC,
                  p.n * unitMs(p.unit) <= absMs,
                  absMs < (p.n + 1) * unitMs(p.unit),
                }
            }
          }
      })

  // The `FromStr` error text quotes the supported formats using these names, and
  // the CLI echoes a chosen encoding with `Display`, so a mismatch would print a
  // value the tool then refuses to accept.
  /// The name an encoding renders as always parses back to that same encoding.
  val encoding_names_round_trip: bool =
    ledger.nameRenders.foldl(true, (ok, r) =>
      ok and encodingFromStr(r.output) == ParsedEncoding(r.input))

  // The INTENT behind `detect`, stated so the simulator produces the witnessing
  // trace instead of the model quietly agreeing with the code. `detect` is a total
  // classifier with no way to signal ambiguity, and no call site tries the other
  // encoding on failure (ethereum lib.rs:150, solana visualsign.rs:80, sui
  // visualsign.rs:46). Four base64 characters carry three bytes and four hex
  // characters carry two, so an ambiguous payload is parsed as a DIFFERENT
  // transaction than the one the wallet sent, and the signer approves that one.
  /// No payload reaching `detect` is well-formed in both base64 and hex.
  val detect_never_faces_an_ambiguous_payload: bool =
    ledger.detects.foldl(true, (ok, r) =>
      ok and not(and { wellFormedBase64(r.input), decodeSucceeded(decodeHex(r.input)) }))
}
