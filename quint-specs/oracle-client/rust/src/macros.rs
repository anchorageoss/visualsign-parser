//! The `log!` macro — the annotation surface that lowers onto the
//! [`Event`](crate::Event) builder.
//!
//! `log!` itself is a cfg-free forwarder, so its documentation lives in one
//! place; the grammar lives in the internal `__log_impl`, whose disabled twin
//! swallows its tokens — production builds compile no logging code and surface
//! no `log!` syntax errors.

/// Log one observation: the action the code under test just took, with its
/// values, post-state assertions, and component scopes.
///
/// The action comes first, a bare identifier. Every other component follows in
/// any order, comma-separated (trailing comma allowed):
///
/// | component | meaning |
/// |---|---|
/// | `%var` | the in-scope variable `var`, logged under its own name |
/// | `name: value` | a named value followed by an in-scope `%var`, a literal, or an `expr` |
/// | `… @ DOMAIN` | after any value: the spec constant this value added to |
/// | `assert!(path, expected)` | post-state assertion: the spec value at `path` now equals `expected` |
/// | `[scope_a, scope_b]` | the component scopes the action belongs to |
///
/// # Values
///
/// Every value arrives named — either the name before `:`, or the variable's
/// own name via `%var`. Replay pins nondeterministic picks by argument name. A
/// nameless value is a compile error.
///
/// ```
/// let amount = 100;
/// let to = "bob";
/// let rate = 3;
///
/// quint_oracle::log!(deposit, amount: 100, [bank]);              // named literal
/// quint_oracle::log!(deposit, %amount, [bank]);                  // variable, own name
/// quint_oracle::log!(deposit, receiver: %to, [bank]);            // variable, renamed
/// quint_oracle::log!(deposit, total: (rate * 100), [bank]);      // computed, parenthesized
/// quint_oracle::log!(deposit, note: format!("n{rate}"), [bank]); // bare expression
/// ```
///
/// # Domains
///
/// `@ DOMAIN` names the spec constant a value is added to; replay collects
/// every value logged under that name into the constant so tests are not
/// limited to hardcoded constants defined in the specification. A *computed*
/// value with a domain needs parens; a literal or `%var` does not.
///
/// ```
/// let from = "alice";
/// let fees = std::collections::BTreeMap::from([("transfer", 1)]);
///
/// quint_oracle::log!(transfer, %from @ ACCOUNTS, [bank]);
/// quint_oracle::log!(transfer, sender: "alice" @ ACCOUNTS, [bank]);
/// quint_oracle::log!(transfer, fee: (fees["transfer"]) @ AMOUNTS, [bank]);
/// ```
///
/// # Assertions
///
/// `assert!(path, expected)` states what the spec must hold after this action.
/// The path is `/`-separated: the spec variable first, then record fields (bare
/// identifiers) and map keys (`%var`, a literal, or `(expr)`). The expected
/// value is `%var`, a literal, or `(expr)`.
///
/// A map key must be a string or an integer and anything else panics.
///
/// ```
/// let from = "alice";
/// let from_expected = 900;
///
/// quint_oracle::log!(
///     transfer,
///     %from @ ACCOUNTS,
///     assert!(accounts / %from, %from_expected),   // %var key and expected
///     assert!(accounts / "bob", 0),                // literal key and expected
///     assert!(ledger / total / (1 + 1), (2 * 50)), // computed key and expected
///     [bank],
/// );
/// ```
///
/// # Scopes
///
/// Scopes are bare identifiers naming the components the action belongs to. At
/// least one is required.
///
/// ```
/// quint_oracle::log!(transfer, amount: 100, [bank, transfers]);
/// ```
///
/// # When it runs
///
/// Statement-only. No component is evaluated unless the oracle is live
/// ([`enabled`](crate::enabled)). Without the `enabled` *feature* the whole
/// invocation is swallowed at compile time — which also means `log!` syntax
/// errors only surface when compiling with the feature on.
#[cfg_attr(
    feature = "enabled",
    doc = r##"
# Compile-time errors

A nameless value fails to compile:

```compile_fail
quint_oracle::log!(transfer, 100, [bank]);
```

`%` takes a single in-scope variable name:

```compile_fail
struct Tx { from: &'static str }
let tx = Tx { from: "alice" };
quint_oracle::log!(transfer, %tx.from, [bank]);
```

A computed expression with a domain needs parens:

```compile_fail
let (a, b) = (1, 2);
quint_oracle::log!(transfer, total: a + b @ AMOUNTS, [bank]);
```
"##
)]
#[macro_export]
macro_rules! log {
    ($($component:tt)*) => {
        $crate::__log_impl!($($component)*)
    };
}

#[cfg(feature = "enabled")]
#[doc(hidden)]
#[macro_export]
macro_rules! __log_impl {
    ($action:ident $(, $($component:tt)*)?) => {
        if $crate::enabled() {
            let __quint_event = $crate::Event::builder(
                $crate::current_test(),
                ::core::stringify!($action),
            );
            $crate::__log_component!(@ __quint_event; $($($component)*)?);
        }
    };
    ($($bad:tt)*) => {
        ::core::compile_error!(
            "log! starts with the action as a bare identifier, then \
             comma-separated components: log!(action, %var, name: value, \
             assert!(path, expected), [scopes])"
        )
    };
}

/// The disabled twin: swallows the invocation whole, so production builds carry
/// no logging code — and no `log!` syntax checking.
#[cfg(not(feature = "enabled"))]
#[doc(hidden)]
#[macro_export]
macro_rules! __log_impl {
    ($($swallowed:tt)*) => {};
}

// Component muncher: dispatches on each component's leading token (`%` → nondet
// from a variable, `assert !` → assertion, `[` → scopes, `ident :` → named
// value) and chains the matching builder call by rebinding `$ev`. Within the
// named-value rules, every rule that starts a `$value:expr` parse comes after
// all token-lookahead rules, so rustc never hard-aborts inside an expression
// fragment for inputs an earlier rule owns.
#[cfg(feature = "enabled")]
#[doc(hidden)]
#[macro_export]
macro_rules! __log_component {
    // Every component chained: post the observation.
    (@ $ev:ident;) => {
        $ev.send();
    };

    // %var [@ DOMAIN]
    (@ $ev:ident; % $var:ident @ $domain:ident $(, $($rest:tt)*)?) => {
        let $ev = $ev.argument(
            ::core::stringify!($var),
            &$var,
            ::core::option::Option::Some(::core::stringify!($domain)),
        );
        $crate::__log_component!(@ $ev; $($($rest)*)?);
    };
    (@ $ev:ident; % $var:ident $(, $($rest:tt)*)?) => {
        let $ev = $ev.argument(
            ::core::stringify!($var),
            &$var,
            ::core::option::Option::None,
        );
        $crate::__log_component!(@ $ev; $($($rest)*)?);
    };
    (@ $ev:ident; % $($bad:tt)*) => {
        ::core::compile_error!(
            "`%` takes a single in-scope variable name, optionally `@ DOMAIN` \
             (a bare constant name); for a field or expression, name it: \
             `name: (expr)`"
        )
    };

    // assert!(path, expected)
    (@ $ev:ident; assert ! ( $($assertion:tt)+ ) $(, $($rest:tt)*)?) => {
        $crate::__log_assert!(@ $ev; [] $($assertion)+);
        $crate::__log_component!(@ $ev; $($($rest)*)?);
    };
    (@ $ev:ident; assert ! $($bad:tt)*) => {
        ::core::compile_error!(
            "assert! takes a path and an expected value: \
             assert!(variable / field / %key, expected)"
        )
    };

    // [scope, …]
    (@ $ev:ident; [ $($scope:ident),+ $(,)? ] $(, $($rest:tt)*)?) => {
        $(let $ev = $ev.scope(::core::stringify!($scope));)+
        $crate::__log_component!(@ $ev; $($($rest)*)?);
    };
    (@ $ev:ident; [ $($bad:tt)* ] $($rest:tt)*) => {
        ::core::compile_error!(
            "scopes are a bracketed list of bare identifiers: [bank, transfers]"
        )
    };

    // name: value [@ DOMAIN]
    (@ $ev:ident; $name:ident : % $var:ident @ $domain:ident $(, $($rest:tt)*)?) => {
        let $ev = $ev.argument(
            ::core::stringify!($name),
            &$var,
            ::core::option::Option::Some(::core::stringify!($domain)),
        );
        $crate::__log_component!(@ $ev; $($($rest)*)?);
    };
    (@ $ev:ident; $name:ident : % $var:ident $(, $($rest:tt)*)?) => {
        let $ev = $ev.argument(
            ::core::stringify!($name),
            &$var,
            ::core::option::Option::None,
        );
        $crate::__log_component!(@ $ev; $($($rest)*)?);
    };
    (@ $ev:ident; $name:ident : % $($bad:tt)*) => {
        ::core::compile_error!(
            "`%` takes a single in-scope variable name, optionally `@ DOMAIN` \
             (a bare constant name); for a field or expression, name it: \
             `name: (expr)`"
        )
    };
    (@ $ev:ident; $name:ident : $value:literal @ $domain:ident $(, $($rest:tt)*)?) => {
        let $ev = $ev.argument(
            ::core::stringify!($name),
            &$value,
            ::core::option::Option::Some(::core::stringify!($domain)),
        );
        $crate::__log_component!(@ $ev; $($($rest)*)?);
    };
    (@ $ev:ident; $name:ident : $value:literal $(, $($rest:tt)*)?) => {
        let $ev = $ev.argument(
            ::core::stringify!($name),
            &$value,
            ::core::option::Option::None,
        );
        $crate::__log_component!(@ $ev; $($($rest)*)?);
    };
    (@ $ev:ident; $name:ident : ( $value:expr ) @ $domain:ident $(, $($rest:tt)*)?) => {
        let $ev = $ev.argument(
            ::core::stringify!($name),
            &($value),
            ::core::option::Option::Some(::core::stringify!($domain)),
        );
        $crate::__log_component!(@ $ev; $($($rest)*)?);
    };
    (@ $ev:ident; $name:ident : ( $value:expr ) $(, $($rest:tt)*)?) => {
        let $ev = $ev.argument(
            ::core::stringify!($name),
            &($value),
            ::core::option::Option::None,
        );
        $crate::__log_component!(@ $ev; $($($rest)*)?);
    };
    (@ $ev:ident; $name:ident : $value:expr $(, $($rest:tt)*)?) => {
        let $ev = $ev.argument(
            ::core::stringify!($name),
            &($value),
            ::core::option::Option::None,
        );
        $crate::__log_component!(@ $ev; $($($rest)*)?);
    };
    (@ $ev:ident; $name:ident : $($bad:tt)*) => {
        ::core::compile_error!(
            "expected a value after `name:` — `%var`, a literal, or `(expr)`, \
             each optionally `@ DOMAIN`; a computed expression with a domain \
             needs the parens: `name: (expr) @ DOMAIN`"
        )
    };

    // Anything else is an unnamed value (or stray tokens).
    (@ $ev:ident; $($bad:tt)+) => {
        ::core::compile_error!(
            "every logged value must be named — `%var` or `name: value` — \
             because replay pins nondet picks by argument name; a nameless \
             value can pin nothing"
        )
    };
}

// Assertion-path muncher: `[…]` accumulates built path segment expressions; a
// `/` continues the path, a `,` ends it and hands the rest to the
// expected-value finisher.
#[cfg(feature = "enabled")]
#[doc(hidden)]
#[macro_export]
macro_rules! __log_assert {
    (@ $ev:ident; [$($seg:expr,)*] $field:ident / $($rest:tt)+) => {
        $crate::__log_assert!(
            @ $ev;
            [$($seg,)* $crate::PathSeg::ident(::core::stringify!($field)),]
            $($rest)+
        );
    };
    (@ $ev:ident; [$($seg:expr,)*] $field:ident, $($expected:tt)+) => {
        $crate::__log_assert_expected!(
            @ $ev;
            [$($seg,)* $crate::PathSeg::ident(::core::stringify!($field)),]
            $($expected)+
        );
    };
    (@ $ev:ident; [$($seg:expr,)*] % $var:ident / $($rest:tt)+) => {
        $crate::__log_assert!(
            @ $ev;
            [$($seg,)* $crate::PathSeg::value(&$var),]
            $($rest)+
        );
    };
    (@ $ev:ident; [$($seg:expr,)*] % $var:ident, $($expected:tt)+) => {
        $crate::__log_assert_expected!(
            @ $ev;
            [$($seg,)* $crate::PathSeg::value(&$var),]
            $($expected)+
        );
    };
    (@ $ev:ident; [$($seg:expr,)*] $key:literal / $($rest:tt)+) => {
        $crate::__log_assert!(
            @ $ev;
            [$($seg,)* $crate::PathSeg::value(&$key),]
            $($rest)+
        );
    };
    (@ $ev:ident; [$($seg:expr,)*] $key:literal, $($expected:tt)+) => {
        $crate::__log_assert_expected!(
            @ $ev;
            [$($seg,)* $crate::PathSeg::value(&$key),]
            $($expected)+
        );
    };
    (@ $ev:ident; [$($seg:expr,)*] ( $key:expr ) / $($rest:tt)+) => {
        $crate::__log_assert!(
            @ $ev;
            [$($seg,)* $crate::PathSeg::value(&($key)),]
            $($rest)+
        );
    };
    (@ $ev:ident; [$($seg:expr,)*] ( $key:expr ), $($expected:tt)+) => {
        $crate::__log_assert_expected!(
            @ $ev;
            [$($seg,)* $crate::PathSeg::value(&($key)),]
            $($expected)+
        );
    };
    (@ $ev:ident; [$($seg:expr,)*] $($bad:tt)*) => {
        ::core::compile_error!(
            "assert! paths are `/`-separated — the spec variable first, then \
             record fields (bare identifiers) and map keys (`%var`, a literal, \
             or `(expr)`) — followed by `, expected`"
        )
    };
}

#[cfg(feature = "enabled")]
#[doc(hidden)]
#[macro_export]
macro_rules! __log_assert_expected {
    (@ $ev:ident; [$($seg:expr,)*] % $var:ident) => {
        let $ev = $ev.assert(
            ::std::vec![$($seg),*],
            &$var,
        );
    };
    (@ $ev:ident; [$($seg:expr,)*] $expected:expr) => {
        let $ev = $ev.assert(
            ::std::vec![$($seg),*],
            &($expected),
        );
    };
    (@ $ev:ident; [$($seg:expr,)*] $($bad:tt)*) => {
        ::core::compile_error!(
            "assert!'s expected value is `%var`, a literal, or `(expr)`"
        )
    };
}
