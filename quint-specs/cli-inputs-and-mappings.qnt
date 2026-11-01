// -*- mode: Bluespec; -*-
/// CLI Inputs and Mappings -- how `parser_app`'s command line is admitted, how
/// each admitted token maps to the option the server actually starts on, and
/// which gates refuse to start.
///
/// Modeled from src/parser/app/src/cli.rs:
///   * `ParserParser::parser()` declares the token set. `--host-ip` and
///     `--host-port` are `takes_value(true).required(true)`; `--ephemeral-file`
///     is `takes_value(true).default_value(EPHEMERAL_KEY_FILE)`, so it always
///     resolves whether or not argv states it; the ABI-posture flag
///     (`--accept-unsigned-abis`) states the trust posture.
///   * `OptionsParser::<ParserParser>::parse` admits argv. An argument outside
///     the declared set is `UnexpectedInput` and is reached BEFORE the
///     required-token check.
///   * `ParserOpts` maps admitted tokens to options: `ip()`/`port()` feed
///     `host_addr()`, which runs `Ipv4Addr::from_str` and `parse::<u16>()`;
///     `ephemeral_file()` returns the stated path or the compiled-in default;
///     `abi_trust()` resolves the trust posture.
///   * `Cli::execute` short-circuits on `--version`/`--help`, otherwise
///     resolves the ABI trust posture -- refusing to start without one --
///     builds the ephemeral key handle, and hands `host_addr()` to
///     `Host::listen`.
///
/// WHAT THIS COVERS
///   * the admission taxonomy: which argv shapes are accepted, which are
///     rejected as unexpected, which are rejected for a missing required token
///   * the token -> option mapping, including the one token whose absence is
///     NOT a failure because it carries a default
///   * the gates between an admitted argv and a listening server, in the order
///     `Cli::execute` reaches them: trust posture, ephemeral key handle,
///     `host_addr()`
///
/// WHAT THIS DOES NOT COVER
///   * argument VALUES beyond whether they resolve. `host_addr()` turns on
///     whether the ip parses as IPv4 and the port as u16; the ephemeral path
///     turns on whether the file is there. The strings themselves are carried
///     opaquely, which is what the CLI does with them.
///   * `--version` / `--help`, which print and return before any mapping
///   * everything after `Host::listen` accepts a connection -- that is the
///     parse/sign pipeline, a separate component
///   * dev-target and Makefile argument sourcing. Which argv a developer's
///     `make` recipe supplies is upstream of this CLI; this spec models what
///     the CLI does with whatever argv it is handed.
///
/// ASSUMED BASE
///   The ABI trust posture is read from PR #481's base branch
///   (`prs-556-parser-app`, stacked on #454) rather than from `main`, which
///   predates it: the diff context shows `parse(&["--accept-unsigned-abis"])`
///   and `ParserOpts { parsed }.abi_trust()`, and #481's description states
///   that `Cli::execute` refuses to start without a stated posture. The order
///   of the posture gate relative to the ephemeral handle and `host_addr()`
///   follows `Cli::execute`'s own sequence.
///
/// REPLAY SURFACE
///   Every logged argument is readable at the site that reports it: which
///   tokens argv carries and whether each stated value resolves are exactly
///   what `cli.rs`'s tests hand to `OptionsParser::parse` and then assert on.
///   Which admission or refusal an invocation hit is computed from those, never
///   pre-decided by an argument.
module cli_inputs_and_mappings {

  // ----------------------------------------------------------------- domain

  /// The declared token spellings, as `ParserParser::parser()` names them.
  pure val HOST_IP_FLAG: str = "--host-ip"
  pure val HOST_PORT_FLAG: str = "--host-port"
  pure val EPHEMERAL_FLAG: str = "--ephemeral-file"
  pure val ABI_POSTURE_FLAG: str = "--accept-unsigned-abis"

  /// A token argv can carry that the parser does not declare. `--usock` is the
  /// real one: `751118bd` replaced it with the two host tokens, so a caller
  /// still passing it is the `UnexpectedInput` case.
  pure val UNDECLARED_FLAG: str = "--usock"

  pure val ACCEPTED_TOKENS: Set[str] =
    Set(HOST_IP_FLAG, HOST_PORT_FLAG, EPHEMERAL_FLAG, ABI_POSTURE_FLAG)

  /// `required(true)`: leaving either out is a parse error, even though every
  /// other declared token is optional.
  pure val REQUIRED_TOKENS: Set[str] = Set(HOST_IP_FLAG, HOST_PORT_FLAG)

  /// `EPHEMERAL_KEY_FILE`, the compiled-in `default_value` -- what
  /// `ephemeral_file()` returns when argv states no path.
  pure val EPHEMERAL_DEFAULT: str = "EPHEMERAL_KEY_FILE"
  /// A path argv states explicitly, standing for any `--ephemeral-file` value.
  pure val EPHEMERAL_STATED: str = "stated-path"
  pure val EPHEMERAL_UNRESOLVED: str = "none"

  // --------------------------------------------------------------- outcomes

  /// `OptionsParser::parse`'s verdict.
  type Admission =
    | AdmitPending
    | AdmitAccepted
    | AdmitUnexpected
    | AdmitMissingRequired

  /// How one gate between an admitted argv and a listening server came out.
  type Gate =
    | GatePending
    | GatePassed
    | GateRefused

  /// How far the invocation got.
  type Startup =
    | StartupPending
    | StartedListening
    | RefusedArgv
    | RefusedPosture
    | RefusedEphemeral
    | RefusedHostAddr

  /// Where the invocation is in `Cli::execute`'s sequence. Every invocation
  /// returns to `PhaseIdle`, so a trace walks as many invocations as it likes.
  type Phase =
    | PhaseIdle
    | PhaseAdmitted
    | PhaseMapped

  // -------------------------------------------------------------- the state

  /// One invocation: the argv handed to the parser, and everything the CLI
  /// derived from it.
  type Invocation = {
    argv: Set[str],
    admission: Admission,
    posture: Gate,
    ephemeralPath: str,
    hostAddr: Gate,
    startup: Startup,
  }

  pure val NO_INVOCATION: Invocation = {
    argv: Set(),
    admission: AdmitPending,
    posture: GatePending,
    ephemeralPath: EPHEMERAL_UNRESOLVED,
    hostAddr: GatePending,
    startup: StartupPending,
  }

  var phase: Phase
  var current: Invocation
  /// Whether the values the CURRENT argv states resolve: the ip as IPv4 and the
  /// port as u16 for `host_addr()`, and the ephemeral key file being present.
  var hostValuesResolve: bool
  var ephemeralFileExists: bool
  /// Every completed invocation.
  var seen: Set[Invocation]

  action specInit: bool = all {
    phase' = PhaseIdle,
    current' = NO_INVOCATION,
    hostValuesResolve' = false,
    ephemeralFileExists' = false,
    seen' = Set(),
  }

  // -------------------------------------------------------------- mappings

  /// `OptionsParser::<ParserParser>::parse`. An undeclared argument is
  /// `UnexpectedInput` and is reached first, which is why a caller that both
  /// passes `--usock` AND omits the host tokens is told about `--usock`.
  pure def admit(argv: Set[str]): Admission =
    if (argv.exclude(ACCEPTED_TOKENS) != Set()) AdmitUnexpected
    else if (REQUIRED_TOKENS.exclude(argv) != Set()) AdmitMissingRequired
    else AdmitAccepted

  /// `ephemeral_file()`. The one declared token whose absence is not a failure:
  /// `default_value(EPHEMERAL_KEY_FILE)` means it always resolves to something.
  pure def ephemeralFileOf(argv: Set[str]): str =
    if (argv.contains(EPHEMERAL_FLAG)) EPHEMERAL_STATED else EPHEMERAL_DEFAULT

  /// `abi_trust()` as `Cli::execute` consumes it: a posture has to be stated,
  /// and the server does not start on a posture that was never resolved.
  pure def postureOf(argv: Set[str]): Gate =
    if (argv.contains(ABI_POSTURE_FLAG)) GatePassed else GateRefused

  /// `host_addr()`: `Ipv4Addr::from_str` on the ip and `parse::<u16>()` on the
  /// port. Both tokens are required, so an admitted argv always carries them --
  /// what is left is whether their values resolve.
  pure def hostAddrOf(valuesResolve: bool): Gate =
    if (valuesResolve) GatePassed else GateRefused

  // ---------------------------------------------------------------- actions

  /// Hand argv to `OptionsParser::<ParserParser>::parse`.
  ///
  /// `statesUndeclared` is a caller still passing `--usock`; the two host bools
  /// are the required tokens; `statesEphemeral` and `statesPosture` are the
  /// optional ones. `hostValuesParse` and `ephemeralPresent` are the values
  /// behind the tokens, carried now because the parser stores them and the
  /// mapping reads them later.
  action parseArgv(
    statesHostIp: bool,
    statesHostPort: bool,
    statesEphemeral: bool,
    statesPosture: bool,
    statesUndeclared: bool,
    hostValuesParse: bool,
    ephemeralPresent: bool,
  ): bool = {
    val argv: Set[str] =
      (if (statesHostIp) Set(HOST_IP_FLAG) else Set())
        .union(if (statesHostPort) Set(HOST_PORT_FLAG) else Set())
        .union(if (statesEphemeral) Set(EPHEMERAL_FLAG) else Set())
        .union(if (statesPosture) Set(ABI_POSTURE_FLAG) else Set())
        .union(if (statesUndeclared) Set(UNDECLARED_FLAG) else Set())
    all {
      phase == PhaseIdle,
      phase' = PhaseAdmitted,
      current' = { ...NO_INVOCATION, argv: argv, admission: admit(argv) },
      hostValuesResolve' = hostValuesParse,
      ephemeralFileExists' = ephemeralPresent,
      seen' = seen,
    }
  }

  /// A rejected argv never reaches any mapping: `ParserOpts::new` panics on the
  /// parse error, so no option is ever resolved from it.
  action abortRejectedArgv: bool = {
    val done: Invocation = { ...current, startup: RefusedArgv }
    all {
      phase == PhaseAdmitted,
      current.admission != AdmitAccepted,
      phase' = PhaseIdle,
      current' = done,
      seen' = seen.union(Set(done)),
      hostValuesResolve' = hostValuesResolve,
      ephemeralFileExists' = ephemeralFileExists,
    }
  }

  /// `Cli::execute` resolves the options off an admitted argv: the trust
  /// posture, the ephemeral key path, and the listen address.
  action mapAdmittedArgv: bool = all {
    phase == PhaseAdmitted,
    current.admission == AdmitAccepted,
    phase' = PhaseMapped,
    current' = { ...current,
      posture: postureOf(current.argv),
      ephemeralPath: ephemeralFileOf(current.argv),
      hostAddr: hostAddrOf(hostValuesResolve),
    },
    hostValuesResolve' = hostValuesResolve,
    ephemeralFileExists' = ephemeralFileExists,
    seen' = seen,
  }

  /// The gates, in `Cli::execute`'s order: refuse without a stated posture,
  /// then build the ephemeral key handle, then parse the listen address and
  /// hand it to `Host::listen`.
  action finishStartup: bool = {
    val outcome: Startup =
      if (current.posture == GateRefused) RefusedPosture
      else if (not(ephemeralFileExists)) RefusedEphemeral
      else if (current.hostAddr == GateRefused) RefusedHostAddr
      else StartedListening
    val done: Invocation = { ...current, startup: outcome }
    all {
      phase == PhaseMapped,
      phase' = PhaseIdle,
      current' = done,
      seen' = seen.union(Set(done)),
      hostValuesResolve' = hostValuesResolve,
      ephemeralFileExists' = ephemeralFileExists,
    }
  }

  // ----------------------------------------------------- behavior observations

  /// An argv carrying a token the parser does not declare was rejected as
  /// unexpected -- the `--usock` shape.
  val UndeclaredArgumentRejected: bool =
    seen.exists(i => i.admission == AdmitUnexpected)

  /// An argv whose every token is declared was still rejected, for leaving out
  /// a `required(true)` host token.
  val RequiredTokenMissingRejected: bool =
    seen.exists(i => i.admission == AdmitMissingRequired)

  /// An invocation resolved its ephemeral key path from the compiled-in
  /// `default_value` rather than from argv.
  val EphemeralPathDefaulted: bool =
    seen.exists(i => i.startup != RefusedArgv and i.ephemeralPath == EPHEMERAL_DEFAULT)

  /// An invocation stated its own ephemeral key path.
  val EphemeralPathStated: bool =
    seen.exists(i => i.ephemeralPath == EPHEMERAL_STATED)

  /// An argv the parser admitted was refused later for stating no ABI trust
  /// posture. Parsing cleanly is not enough to start.
  val PostureRefusedAdmittedArgv: bool =
    seen.exists(i => i.startup == RefusedPosture)

  /// An admitted argv stating a posture was refused on the address it carried:
  /// the ip or port value did not resolve for `host_addr()`.
  val HostAddressValueRefused: bool =
    seen.exists(i => i.startup == RefusedHostAddr)

  /// The server reached `Host::listen`.
  val ServerStartedListening: bool =
    seen.exists(i => i.startup == StartedListening)

  // ------------------------------------------------------------- invariants

  /// The trust posture is a startup gate, not advice: nothing listens without
  /// one stated.
  val neverListensWithoutStatedPosture: bool =
    seen.forall(i =>
      i.startup != StartedListening or i.argv.contains(ABI_POSTURE_FLAG))

  /// `required(true)` on both host tokens means an admitted argv always carries
  /// both -- so `ip()` and `port()` never look for a token that is not there.
  val admittedArgvCarriesBothHostTokens: bool =
    seen.forall(i =>
      i.admission != AdmitAccepted or REQUIRED_TOKENS.subseteq(i.argv))

  /// `default_value` means the ephemeral key path always resolves to something
  /// once argv is admitted, stated or not.
  val ephemeralPathAlwaysResolvesOnceAdmitted: bool =
    seen.forall(i =>
      i.admission != AdmitAccepted or i.ephemeralPath != EPHEMERAL_UNRESOLVED)

  /// A rejected argv resolves no option at all: `ParserOpts::new` panics before
  /// any mapping runs.
  val rejectedArgvMapsNothing: bool =
    seen.forall(i =>
      i.admission == AdmitAccepted or and {
        i.startup == RefusedArgv,
        i.posture == GatePending,
        i.hostAddr == GatePending,
        i.ephemeralPath == EPHEMERAL_UNRESOLVED,
      })

  /// Only an undeclared argument is reported as unexpected; a missing required
  /// token is its own rejection.
  val unexpectedOnlyForUndeclaredTokens: bool =
    seen.forall(i =>
      i.admission != AdmitUnexpected or i.argv.exclude(ACCEPTED_TOKENS) != Set())
}

/// Oracle main: replay entry point. Every nondet pick is branch-local and named
/// exactly as the argument the instrumented code reads at that site.
module CliInputsAndMappingsOracle {
  import cli_inputs_and_mappings.*

  action init: bool = specInit

  action step: bool = any {
    {
      nondet statesHostIp = Bool.oneOf()
      nondet statesHostPort = Bool.oneOf()
      nondet statesEphemeral = Bool.oneOf()
      nondet statesPosture = Bool.oneOf()
      nondet statesUndeclared = Bool.oneOf()
      nondet hostValuesParse = Bool.oneOf()
      nondet ephemeralPresent = Bool.oneOf()
      parseArgv(statesHostIp, statesHostPort, statesEphemeral, statesPosture,
                statesUndeclared, hostValuesParse, ephemeralPresent)
    },
    abortRejectedArgv,
    mapAdmittedArgv,
    finishStartup,
  }
}

module CliInputsAndMappingsTest {
  import cli_inputs_and_mappings.*

  action init: bool = specInit

  /// Both host tokens, a stated posture, values that resolve, and no
  /// `--ephemeral-file` -- so the path comes from `default_value`. Listens.
  run defaultedEphemeralPathStartsTest =
    specInit
      .then(parseArgv(true, true, false, true, false, true, true))
      .then(mapAdmittedArgv)
      .then(finishStartup)
      .expect(and {
        current.admission == AdmitAccepted,
        current.ephemeralPath == EPHEMERAL_DEFAULT,
        current.startup == StartedListening,
        EphemeralPathDefaulted,
        ServerStartedListening,
        neverListensWithoutStatedPosture,
      })

  /// The same invocation with `--ephemeral-file` stated: the stated path wins
  /// over the default, and nothing else about startup changes.
  run statedEphemeralPathStartsTest =
    specInit
      .then(parseArgv(true, true, true, true, false, true, true))
      .then(mapAdmittedArgv)
      .then(finishStartup)
      .expect(and {
        current.ephemeralPath == EPHEMERAL_STATED,
        current.startup == StartedListening,
        EphemeralPathStated,
      })

  /// A caller still passing `--usock` after `751118bd` removed it. Rejected as
  /// unexpected -- and rejected BEFORE the two host tokens it is also missing,
  /// so the failure names `--usock`. No option is resolved.
  run undeclaredArgumentRejectedTest =
    specInit
      .then(parseArgv(false, false, true, true, true, true, true))
      .then(abortRejectedArgv)
      .expect(and {
        current.admission == AdmitUnexpected,
        current.startup == RefusedArgv,
        UndeclaredArgumentRejected,
        rejectedArgvMapsNothing,
        unexpectedOnlyForUndeclaredTokens,
      })

  /// Every token declared, but `--host-port` left out. A different rejection
  /// from the one above, and still nothing mapped.
  run requiredTokenMissingRejectedTest =
    specInit
      .then(parseArgv(true, false, false, true, false, true, true))
      .then(abortRejectedArgv)
      .expect(and {
        current.admission == AdmitMissingRequired,
        RequiredTokenMissingRejected,
        rejectedArgvMapsNothing,
      })

  /// An argv that parses cleanly and states no posture. Parsing is not the
  /// gate: `Cli::execute` resolves the posture and refuses to start.
  run admittedArgvWithoutPostureRefusedTest =
    specInit
      .then(parseArgv(true, true, false, false, false, true, true))
      .then(mapAdmittedArgv)
      .then(finishStartup)
      .expect(and {
        current.admission == AdmitAccepted,
        current.posture == GateRefused,
        current.startup == RefusedPosture,
        PostureRefusedAdmittedArgv,
        neverListensWithoutStatedPosture,
      })

  /// Every token right and a stated posture, but the address VALUES do not
  /// resolve -- `host_addr()` refuses past both earlier gates.
  run hostAddressValueRefusedTest =
    specInit
      .then(parseArgv(true, true, false, true, false, false, true))
      .then(mapAdmittedArgv)
      .then(finishStartup)
      .expect(and {
        current.admission == AdmitAccepted,
        current.posture == GatePassed,
        current.startup == RefusedHostAddr,
        HostAddressValueRefused,
      })

  /// Three invocations back to back: one refused on argv, one refused on
  /// posture, one that listens. Each returns to idle, so the mapping is
  /// per-invocation and carries nothing over.
  run repeatedInvocationsTest =
    specInit
      .then(parseArgv(false, false, false, true, true, true, true))
      .then(abortRejectedArgv)
      .expect(current.startup == RefusedArgv)
      .then(parseArgv(true, true, false, false, false, true, true))
      .then(mapAdmittedArgv)
      .then(finishStartup)
      .expect(current.startup == RefusedPosture)
      .then(parseArgv(true, true, true, true, false, true, true))
      .then(mapAdmittedArgv)
      .then(finishStartup)
      .expect(and {
        current.startup == StartedListening,
        UndeclaredArgumentRejected,
        PostureRefusedAdmittedArgv,
        ServerStartedListening,
        EphemeralPathStated,
        rejectedArgvMapsNothing,
        admittedArgvCarriesBothHostTokens,
        ephemeralPathAlwaysResolvesOnceAdmitted,
      })
}
