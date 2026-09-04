//! CLI for the parser app
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    str::FromStr,
};

use qos_core::{
    EPHEMERAL_KEY_FILE,
    cli::EPHEMERAL_FILE_OPT,
    handles::EphemeralKeyHandle,
    parser::{GetParserForOptions, OptionsParser, Parser, Token},
};
use visualsign::signing::MetadataTrustPolicy;

use crate::config::ParserConfig;
use crate::payment_verify::PaymentPolicy;

const HOST_IP: &str = "host-ip";
const HOST_PORT: &str = "host-port";
const ACCEPT_UNSIGNED_ABIS: &str = "accept-unsigned-abis";
const ACCEPT_SIGNATURES_FROM_PUBKEY: &str = "accept-signatures-from-pubkey";

/// CLI options for starting up the app server.
#[derive(Default, Clone, Debug, PartialEq)]
pub struct ParserOpts {
    parsed: Parser,
}

impl ParserOpts {
    fn new(args: &mut Vec<String>) -> Self {
        let parsed = OptionsParser::<ParserParser>::parse(args)
            .unwrap_or_else(|e| panic!("Parser: invalid CLI args: {e}"));

        Self { parsed }
    }

    /// Address the host server should listen on.
    fn host_addr(&self) -> SocketAddr {
        let ip = Ipv4Addr::from_str(&self.ip()).expect("could not parse ip to IP v4");
        let port = self
            .port()
            .parse::<u16>()
            .expect("could not parse port to u16");
        SocketAddr::new(IpAddr::V4(ip), port)
    }

    fn ip(&self) -> String {
        self.parsed
            .single(HOST_IP)
            .expect("host ip required")
            .clone()
    }

    fn port(&self) -> String {
        self.parsed
            .single(HOST_PORT)
            .expect("host port required")
            .clone()
    }

    fn ephemeral_file(&self) -> String {
        self.parsed
            .single(EPHEMERAL_FILE_OPT)
            .expect("has a default value.")
            .clone()
    }

    /// The deploy-time trust posture for caller-supplied ABI mappings.
    ///
    /// # Errors
    /// Returns `Err` if the two posture options are combined, if neither is given, or
    /// if a supplied signer public key is invalid. All three are startup failures:
    /// the posture is what a signer verifies against this deployment, so guessing one
    /// would defeat the point.
    fn abi_trust(&self) -> Result<MetadataTrustPolicy, String> {
        ParserConfig::abi_trust_from_options(
            self.parsed.flag(ACCEPT_UNSIGNED_ABIS).unwrap_or(false),
            self.parsed
                .multiple(ACCEPT_SIGNATURES_FROM_PUBKEY)
                .unwrap_or_default(),
        )
    }
}

struct ParserParser;
impl GetParserForOptions for ParserParser {
    fn parser() -> Parser {
        Parser::new()
            .token(
                Token::new(HOST_IP, "IP address this server should listen on")
                    .takes_value(true)
                    .required(true),
            )
            .token(
                Token::new(HOST_PORT, "port this server should listen on")
                    .takes_value(true)
                    .required(true),
            )

            .token(
                Token::new(
                    EPHEMERAL_FILE_OPT,
                    "path to file where the Ephemeral Key secret should be retrieved from. Use default for production.",
                )
                .takes_value(true)
                .default_value(EPHEMERAL_KEY_FILE),
            )
            // The ABI trust posture. Exactly one of the next two options must be
            // given: `forbids` rejects passing both, and `abi_trust()` rejects
            // passing neither (the parser cannot express "one of"). Being on the
            // cmdline is the point: the intended end state is for it to land in the
            // signed deployment manifest's pivotArgs so the posture is auditable out
            // of band, but that wiring has not landed yet (see CLAUDE.md).
            .token(
                Token::new(
                    ACCEPT_UNSIGNED_ABIS,
                    "Required (exactly one of --accept-unsigned-abis / --accept-signatures-from-pubkey): accept caller-supplied ABI mappings that carry no signature. Their integrity and provenance are unverified. Mutually exclusive with --accept-signatures-from-pubkey.",
                )
                .forbids(vec![ACCEPT_SIGNATURES_FROM_PUBKEY]),
            )
            .token(
                Token::new(
                    ACCEPT_SIGNATURES_FROM_PUBKEY,
                    "Required (exactly one of --accept-unsigned-abis / --accept-signatures-from-pubkey): only accept caller-supplied ABI mappings signed by this hex secp256k1 public key; unsigned and otherwise-signed mappings are rejected. Repeatable. Mutually exclusive with --accept-unsigned-abis. Requires a build with the `ethereum` feature (on by default); a build without it refuses to start when this flag is given.",
                )
                .takes_value(true)
                .allow_multiple(true)
                .forbids(vec![ACCEPT_UNSIGNED_ABIS]),
            )
    }
}

/// app cli
pub struct Cli;
impl Cli {
    /// start the parser app
    ///
    /// # Panics
    ///
    /// Panics if the socket server cannot start, or if the ABI trust posture is
    /// missing/contradictory/malformed. The posture is refused rather than defaulted:
    /// coming up in a mode nobody chose is exactly the failure this replaces.
    pub async fn execute() {
        let mut args: Vec<String> = std::env::args().collect();

        let opts = ParserOpts::new(&mut args);

        if opts.parsed.version() {
            println!("version: {}", env!("VERSION"));
        } else if opts.parsed.help() {
            println!("{}", opts.parsed.info());
        } else {
            let abi_trust = match opts.abi_trust() {
                Ok(policy) => policy,
                Err(e) => panic!("Parser: invalid ABI trust configuration: {e}"),
            };
            // No binary wires up a gateway pubkey yet, so payment verification
            // stays disabled here too (see `payment_verify` module doc).
            let config = ParserConfig::new(abi_trust, PaymentPolicy::Disabled);

            println!(
                "---- Starting Parser server (version: {}) -----",
                env!("VERSION")
            );
            println!("caller-supplied ABI trust: {}", config.abi_trust);
            let processor = crate::service::Processor::new(
                EphemeralKeyHandle::new(opts.ephemeral_file()),
                config,
            );

            let mut tasks = Vec::new();
            tasks.push(tokio::spawn(async move {
                crate::host::Host::listen(opts.host_addr(), processor)
                    .await
                    .expect("`AsyncHost::listen` error");
            }));

            match tokio::signal::ctrl_c().await {
                Ok(()) => eprintln!("handling ctrl+c the tokio way"),

                Err(err) => panic!("{err}"),
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A real uncompressed secp256k1 public key (scalar `[0x42; 32]`), so the
    /// require-signed path can decode it.
    const TEST_PUBKEY: &str = "0424653eac434488002cc06bbfb7f10fe18991e35f9fe4302dbea6d2353dc0ab1c119fc5009a032aa9fe47f5e149bb8442f71f884ccb516590686d8ff6ab91c613";

    /// Parse a full argv (minus the binary name, which the parser skips) the way
    /// `ParserOpts::new` does, but surfacing the error instead of panicking.
    fn parse(extra: &[&str]) -> Result<ParserOpts, String> {
        let mut args: Vec<String> = [
            "parser_app",
            "--host-ip",
            "127.0.0.1",
            "--host-port",
            "3000",
        ]
        .iter()
        .chain(extra.iter())
        .map(|s| (*s).to_string())
        .collect();
        OptionsParser::<ParserParser>::parse(&mut args)
            .map(|parsed| ParserOpts { parsed })
            .map_err(|e| format!("{e:?}"))
    }

    #[test]
    fn accept_unsigned_flag_selects_the_permissive_posture() {
        let opts = parse(&["--accept-unsigned-abis"]).expect("valid args");
        let policy = opts.abi_trust().expect("posture resolves");
        assert!(policy.accepts_unsigned());
    }

    #[cfg(feature = "ethereum")]
    #[test]
    fn signer_pubkey_flag_selects_the_strict_posture() {
        let opts = parse(&["--accept-signatures-from-pubkey", TEST_PUBKEY]).expect("valid args");
        let policy = opts.abi_trust().expect("posture resolves");
        assert!(!policy.accepts_unsigned());
        assert_eq!(policy.signer_allowlist().expect("allowlist").len(), 1);
    }

    /// The flag is repeatable, so a deployment can allowlist more than one signer
    /// (e.g. across a key rotation) without a second deploy in between.
    #[cfg(feature = "ethereum")]
    #[test]
    fn signer_pubkey_flag_is_repeatable() {
        const OTHER_PUBKEY: &str = "0479be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798483ada7726a3c4655da4fbfc0e1108a8fd17b448a68554199c47d08ffb10d4b8";
        let opts = parse(&[
            "--accept-signatures-from-pubkey",
            TEST_PUBKEY,
            "--accept-signatures-from-pubkey",
            OTHER_PUBKEY,
        ])
        .expect("valid args");
        let policy = opts.abi_trust().expect("posture resolves");
        assert_eq!(policy.signer_allowlist().expect("allowlist").len(), 2);
    }

    /// The two postures are XOR: the parser itself rejects the combination, before
    /// any posture is resolved.
    #[test]
    fn both_postures_are_rejected_by_the_parser() {
        let err = parse(&[
            "--accept-unsigned-abis",
            "--accept-signatures-from-pubkey",
            TEST_PUBKEY,
        ])
        .expect_err("the two postures are mutually exclusive");
        assert!(err.contains("MutuallyExclusiveInput"), "error: {err}");
    }

    /// Omitting both must fail rather than silently pick one. `qos_core`'s parser
    /// cannot express "exactly one of", so `abi_trust()` enforces the lower bound.
    #[test]
    fn omitting_both_postures_is_an_error() {
        let opts = parse(&[]).expect("args parse; the posture check is separate");
        let err = opts
            .abi_trust()
            .expect_err("a deployment must state its posture");
        assert!(err.contains("is required"), "error: {err}");
    }

    /// The posture flags must not disturb the options the process actually listens
    /// on.
    #[test]
    fn posture_flags_do_not_affect_the_listen_address() {
        let opts = parse(&["--accept-unsigned-abis"]).expect("valid args");
        assert_eq!(opts.host_addr().to_string(), "127.0.0.1:3000");
    }

    /// Extract one target's recipe arguments from a Makefile, as the shell would
    /// receive them after `cargo run --bin <x> --`.
    ///
    /// Deliberately simple: find the target line, take the tab-indented recipe
    /// lines under it, drop everything through the `--` that separates cargo's own
    /// arguments from the binary's, and substitute `$(VAR)` references. It does not
    /// try to be `make` -- if the recipe ever grows conditionals or a `$(shell ...)`
    /// call this stops reflecting reality, which is what the structural assertions
    /// in the caller are there to catch: they fail on an unreadable recipe rather
    /// than letting an empty argument list parse cleanly and prove nothing.
    fn makefile_target_args(makefile: &str, target: &str) -> Vec<String> {
        let mut recipe = String::new();
        let mut in_target = false;
        for line in makefile.lines() {
            if in_target {
                // The recipe ends at the first line that is not tab-indented.
                let Some(body) = line.strip_prefix('\t') else {
                    break;
                };
                recipe.push(' ');
                recipe.push_str(body.trim_end().trim_end_matches('\\'));
            } else if line.starts_with(&format!("{target}:")) {
                in_target = true;
            }
        }

        // `--` separates cargo's arguments from the binary's; everything before it
        // (`cargo run --bin parser_app`) is not the binary's argv.
        let after_separator = recipe
            .split_whitespace()
            .skip_while(|token| *token != "--")
            .skip(1);

        after_separator.map(substitute_make_vars).collect()
    }

    /// Replace every `$(VAR)` in one token with a value the CLI will accept. The
    /// two that have to look real are the ones `host_addr()` parses; anything else
    /// is only carried as an opaque string.
    fn substitute_make_vars(token: &str) -> String {
        let mut out = String::with_capacity(token.len());
        let mut rest = token;
        while let Some(start) = rest.find("$(") {
            out.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            let Some(end) = after.find(')') else {
                // Unbalanced `$(`: leave it verbatim rather than guessing, so the
                // caller's parse fails loudly instead of on a silently mangled arg.
                break;
            };
            out.push_str(match &after[..end] {
                "PARSER_HOST" => "127.0.0.1",
                "PARSER_PORT" => "3000",
                _ => "placeholder",
            });
            rest = &after[end + 1..];
        }
        out.push_str(rest);
        out
    }

    /// Guards `make -C src parser_app` against argument drift in either direction.
    ///
    /// Nothing in CI runs the dev `make` targets -- `make {generated,lint,test,
    /// narrow-build-check}` is the whole of it -- so an argument this CLI stops
    /// accepting leaves the target broken with every check green. That is not
    /// hypothetical: the target passed `--usock` for some time after the flag was
    /// replaced by `--host-ip`/`--host-port`, and panicked before reaching any of
    /// the flags this module's other tests cover.
    ///
    /// Feeding the Makefile's own arguments to the real parser catches both
    /// directions: a flag removed from the CLI, and a stale flag left in the
    /// Makefile. `include_str!` makes the Makefile a compile-time dependency, so
    /// moving or renaming it is a build failure rather than a test that quietly
    /// stops checking anything.
    #[test]
    fn the_makefile_dev_target_args_are_accepted_by_this_cli() {
        let makefile = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Makefile"));
        let args = makefile_target_args(makefile, "parser_app");

        // A recipe this extractor cannot read would make the parse below vacuous:
        // an empty argument list parses fine and proves nothing. Pin only the
        // structure -- some arguments, at least one of them a flag -- and
        // deliberately not *which* flags, so a legitimate rename does not fail
        // here with a misleading "the extractor broke" message instead of
        // reaching the parse that is the actual subject of this test.
        assert!(
            !args.is_empty(),
            "extracted no arguments from the parser_app target; the recipe shape \
             changed and this guard is no longer reading it"
        );
        assert!(
            args.iter().any(|arg| arg.starts_with("--")),
            "extracted no flags from the parser_app target, only {args:?}; the \
             recipe shape changed and this guard is no longer reading it"
        );

        // `OptionsParser` skips the leading program name, as it does for real argv.
        let mut with_program_name = vec!["parser_app".to_string()];
        with_program_name.extend(args.iter().cloned());
        let parsed =
            OptionsParser::<ParserParser>::parse(&mut with_program_name).unwrap_or_else(|e| {
                panic!(
                    "`make -C src parser_app` passes arguments this CLI rejects, so the \
                 target cannot start: {e:?}. Extracted args: {args:?}"
                )
            });

        // Parsing alone would still pass if the target lost its posture flag, which
        // `Cli::execute` refuses to start without. Resolve it too, so the guard
        // covers the whole startup path the target actually takes.
        ParserOpts { parsed }
            .abi_trust()
            .unwrap_or_else(|e| panic!("the parser_app target states no usable ABI posture: {e}"));
    }
}
