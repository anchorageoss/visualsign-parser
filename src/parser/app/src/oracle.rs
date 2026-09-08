//! Quint Studio behavior-coverage instrumentation for the parse/sign pipeline.
//!
//! Each function reports one transition of the `parse-sign-pipeline` Quint spec
//! (`quint-specs/parse-sign-pipeline.qnt`) to the Quint oracle daemon. Action names
//! and argument names are copied verbatim from that spec — the oracle replays the
//! logged sequence against it, so they must match exactly.
//!
//! Everything here compiles to nothing unless the crate is built with the `oracle`
//! feature, which is OFF by default: the production enclave binary must not link an
//! HTTP client. Even with the feature on, the client is inert unless the oracle set
//! `QUINT_ORACLE_URL` for the process it spawned.

/// The Quint Studio component key these events belong to — the `eventScope` of
/// `quint-oracle.parse-sign-pipeline.json`. Events carrying it are the only ones
/// this component's oracle replays.
#[cfg(feature = "oracle")]
const COMPONENT: &str = "parse-sign-pipeline";

/// The oracle test boundary: one trace per test. Bind it with
/// `let _t = oracle::start_test("<test name>");` and hold it for the test's scope.
#[cfg(feature = "oracle")]
pub(crate) type TestGuard = quint_oracle::TestGuard;

/// Inert stand-in so call sites need no `cfg` of their own.
#[cfg(not(feature = "oracle"))]
pub(crate) struct TestGuard;

/// Begin an oracle trace named `name`. No-op (and inert guard) unless the `oracle`
/// feature is on and the oracle set `QUINT_ORACLE_URL`.
#[cfg(feature = "oracle")]
pub(crate) fn start_test(name: &str) -> TestGuard {
    quint_oracle::register_test(name)
}

#[cfg(not(feature = "oracle"))]
pub(crate) fn start_test(_name: &str) -> TestGuard {
    TestGuard
}

/// Report a transition that takes no arguments in the spec.
#[cfg(feature = "oracle")]
pub(crate) fn event(action: &'static str) {
    quint_oracle::Event::builder(quint_oracle::current_test(), action)
        .scope(COMPONENT)
        .send();
}

/// Report `dispatch_to_converter`: the request cleared every pre-check, the
/// production options are built, and it is going to the registry.
///
/// `chain` is the proto enum's `as_str_name()` spelling, which is what the spec's
/// `CHAIN_NAMES` domain contains.
#[cfg(feature = "oracle")]
pub(crate) fn dispatched(chain: &str, has_metadata: bool, include_intermediate: bool) {
    quint_oracle::Event::builder(quint_oracle::current_test(), "dispatch_to_converter")
        .argument("chain", chain, Some("CHAIN_NAMES"))
        .argument("hasMetadata", has_metadata, None)
        .argument("includeIntermediate", include_intermediate, None)
        .scope(COMPONENT)
        .send();
}

/// Report `conversion_succeeded`, carrying whether the converter returned any
/// `intermediate_output` bytes.
#[cfg(feature = "oracle")]
pub(crate) fn converted(has_intermediate: bool) {
    quint_oracle::Event::builder(quint_oracle::current_test(), "conversion_succeeded")
        .argument("hasIntermediate", has_intermediate, None)
        .scope(COMPONENT)
        .send();
}

#[cfg(not(feature = "oracle"))]
pub(crate) fn event(_action: &str) {}

#[cfg(not(feature = "oracle"))]
pub(crate) fn dispatched(_chain: &str, _has_metadata: bool, _include_intermediate: bool) {}

#[cfg(not(feature = "oracle"))]
pub(crate) fn converted(_has_intermediate: bool) {}
