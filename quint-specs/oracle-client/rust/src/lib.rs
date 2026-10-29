//! Instrumentation client for the Quint Studio oracle: logs what your tests
//! actually do, so Quint Studio can validate those observations against a Quint
//! specification.
//!
//! # The model
//!
//! An *observation* is one action your code took, logged at the moment it
//! happened: an action name, the values involved (each one named, optionally
//! tagged with the spec constant — its *domain* — the value is added to),
//! optional post-state *assertions* (what a spec variable must now contain),
//! and the component *scopes* the action belongs to. Observations logged during
//! one test form that test's *trace*; the oracle buffers each trace under the
//! test's name and, when the test finishes successfully, replays it against the
//! spec. A test that fails (panics, or returns `Err`) has its trace discarded.
//!
//! Each execution of a test is one *run*: registering the same test name again
//! (a table-driven loop, a parametric harness) opens a fresh run rather than
//! appending to the previous trace.
//!
//! # The no-op guarantee
//!
//! This crate is **inert by default**: all real code sits behind the
//! off-by-default cargo feature `enabled`. Without it, every function is an
//! `#[inline(always)]` empty stub and no dependencies are compiled — a
//! production `cargo build`/`--release` carries **zero oracle code**.
//!
//! # Registering tests
//!
//! The first observation logged on a test's thread registers that test — the
//! Rust test harness names each test's thread after the test — and its outcome
//! is reported when the thread exits, via a panic hook this crate chains onto
//! the global one.
//!
//! ```
//! // Inside a plain `#[test] fn transfer_settles()`, with no annotation at
//! // all: this call registers `transfer_settles` and reports its outcome.
//! quint_oracle::log!(transfer, amount: 100, [bank]);
//! ```
//!
//! That default cannot see everything. Reach for an escape hatch when it
//! cannot:
//!
//! | when | use |
//! |---|---|
//! | observations come from spawned or runtime worker threads | [`quint_oracle::test`](test) |
//! | the test reports failure by returning `Err`, not panicking | [`quint_oracle::test`](test) |
//! | you have replaced the global panic hook yourself | [`quint_oracle::test`](test) |
//! | the test thread cannot supply a valid test name | [`quint_oracle::register_test`](register_test) |
//!
//! Both hatches take precedence over lazy registration, and both report exactly
//! one outcome per run. [`register_test`] returns a [`TestGuard`] that reports
//! on drop; keep it alive for the whole test with `let _guard = …`.
//!
//! Do not instrument `#[should_panic]` tests: the intended panic reports the
//! run as failed.
//!
//! # Logging observations
//!
//! Everything goes through [`log!`]: the action first, then named values,
//! assertions, and scopes.
//!
//! ```
//! quint_oracle::log!(deposit, amount: 100, [bank]);
//! ```
//!
//! `%name` logs an in-scope variable under its own name:
//!
//! ```
//! let amount = 100;
//! let account = "alice";
//!
//! quint_oracle::log!(deposit, %account, %amount, [bank]);
//! ```
//!
//! `@ DOMAIN` tags a value with the spec constant it's value is appended to.
//! Replay collects every value logged under `ACCOUNTS` into that constant,
//! which is how the spec learns your test's actual account names:
//!
//! ```
//! let account = "alice";
//! quint_oracle::log!(deposit, %account @ ACCOUNTS, amount: 100, [bank]);
//! ```
//!
//! Values don't need to be variables. A literal stands for itself, `(expr)` is
//! computed, and a bare expression works when it carries no domain:
//!
//! ```
//! let fees = std::collections::BTreeMap::from([("transfer", 1)]);
//!
//! quint_oracle::log!(
//!     transfer,
//!     sender: "alice" @ ACCOUNTS,        // named literal
//!     fee: (fees["transfer"]) @ AMOUNTS, // computed: parens required with @
//!     note: format!("tx-{}", 1),         // bare expression, no domain
//!     [bank],
//! );
//! ```
//!
//! `assert!(path, expected)` pins the post-state: after this action, the spec
//! value at `path` must equal `expected`:
//!
//! ```
//! let from = "alice";
//! let from_expected = 900;
//!
//! quint_oracle::log!(
//!     transfer,
//!     %from @ ACCOUNTS,
//!     assert!(accounts / %from, %from_expected),
//!     [bank],
//! );
//! ```
//!
//! Finally, an action may belong to several component scopes; the daemon needs
//! at least one:
//!
//! ```
//! quint_oracle::log!(transfer, amount: 100, [bank, transfers]);
//! ```
//!
//! [`log!`] documents every form as a reference, including the compile errors.
//!
//! # Values
//!
//! A logged value is anything implementing [`ToLogged`]. Conversions ship for
//! bools, strings (`str` and `String`), every integer type, slices and `Vec`
//! (a Quint list), `BTreeSet`/`HashSet` (a set), `BTreeMap`/`HashMap` (a map),
//! and tuples up to arity 12 whose components convert (a Quint tuple). Element
//! order in a set or a map is not significant — the oracle compares them
//! structurally.
//!
//! [`record`] builds the shape no Rust type maps onto, and [`tuple()`] the
//! tuples the impls above don't reach — wider than arity 12, or assembled at
//! runtime. Both take already-converted values, so a domain type composes its
//! own rendering from the fields' `to_logged()`:
//!
//! ```
//! use quint_oracle::{record, ToLogged, Value};
//!
//! struct Account {
//!     owner: String,
//!     balance: u64,
//! }
//!
//! impl ToLogged for Account {
//!     fn to_logged(&self) -> Value {
//!         record([
//!             ("owner", self.owner.to_logged()),
//!             ("balance", self.balance.to_logged()),
//!         ])
//!     }
//! }
//! ```
//!
//! Heterogeneous sets and maps need no constructor of their own: [`Value`] is
//! itself [`ToLogged`] and `Ord`, so `BTreeSet<Value>` and
//! `BTreeMap<Value, Value>` convert through the impls above.
//!
//! A payload-free Quint variant is logged as its name, a plain string — there
//! are no variant helpers, by design.

/// The oracle wire-protocol version this client speaks.
pub const PROTOCOL_VERSION: u32 = 1;

mod macros;

/// Marks a function as an oracle-instrumented test: registers it with the
/// oracle before the body runs and reports its outcome when it finishes.
///
/// An escape hatch, not the default path — see the table under *Registering
/// tests* for the situations that call for it. Where lazy registration
/// suffices, no annotation is better. What this adds:
///
/// - derives the test's libtest name (module path + function name), so
///   registration never depends on thread names;
///   
/// - pre-registers the test *before the body runs*, which keeps observations
///   from spawned worker threads attributable and works under
///   `--test-threads=1`;
///   
/// - adds `#[test]` for you — unless one is already present, or the function
///   is `async` (an async test keeps its own runtime attribute, placed
///   *below* this one, e.g. `#[tokio::test]`);
///   
/// - reports `failed` when the body panics **or** returns `Err` — failure
///   detection the lazy path cannot match.
///
/// Supported shapes (v1): plain functions, `Result`-returning functions, and
/// async functions whose runtime attribute sits below this one. Not
/// supported: generics, `#[should_panic]` (don't instrument those — the
/// intended panic would report the run as failed), and custom test harnesses.
/// Takes no arguments.
///
/// ```
/// #[quint_oracle::test]
/// fn transfer_settles() {
///     quint_oracle::log!(transfer, amount: 100, [bank]);
/// }
///
/// #[quint_oracle::test]
/// fn checked_transfer() -> Result<(), String> {
///     quint_oracle::log!(transfer, amount: 100, [bank]);
///     Ok(())
/// }
///
/// #[quint_oracle::test]
/// #[tokio::test]
/// async fn async_transfer_settles() {
///     quint_oracle::log!(transfer, amount: 100, [bank]);
/// }
/// ```
pub use quint_oracle_macros::test;

#[cfg(feature = "enabled")]
mod live;
#[cfg(feature = "enabled")]
pub use live::*;

#[cfg(not(feature = "enabled"))]
mod stubs;
#[cfg(not(feature = "enabled"))]
pub use stubs::*;
