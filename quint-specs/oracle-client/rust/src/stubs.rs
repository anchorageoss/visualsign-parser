//! The disabled-build surface: every public item of the enabled crate, as an
//! inert stub, so instrumented code compiles unchanged with the `enabled`
//! feature off and the optimizer erases all of it.
//!
//! [`ToLogged`] mirrors the enabled build's exact impl set (rather than one
//! blanket impl over every type) so trait coherence is identical in both
//! configurations: a customer's own `impl ToLogged for TheirType` must
//! compile with the feature off too.
//!
//! Every item carries a one-line summary, because this is the DEFAULT build:
//! without them, editor hover on any `quint_oracle::` item shows nothing at
//! all. They are summaries only — the documented contract lives on the
//! `enabled` counterpart, so the two cannot drift into disagreeing prose.
//! Signature drift is caught by `tests/surface.rs`, which compiles this
//! surface and the live one against the same calls.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// Stand-in for `itf::Value`; carries nothing.
///
/// The comparison derives mirror `itf::Value`'s: a `ToLogged` impl that builds
/// a `BTreeSet<Value>` or a `BTreeMap<Value, Value>` — the way heterogeneous
/// collections are spelled — must compile in this build too.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Value;

/// Conversion into the logged ITF value dialect. Inert here; the dialect
/// and the contract are documented on the `enabled` counterpart.
pub trait ToLogged {
    fn to_logged(&self) -> Value;
}

macro_rules! impl_stub_to_logged {
    ($($ty:ty),*) => {$(
        impl ToLogged for $ty {
            #[inline(always)]
            fn to_logged(&self) -> Value {
                Value
            }
        }
    )*};
}

impl_stub_to_logged!(
    Value, bool, str, String, i8, i16, i32, i64, u8, u16, u32, isize, usize, u64, i128, u128
);

impl<T: ToLogged + ?Sized> ToLogged for &T {
    #[inline(always)]
    fn to_logged(&self) -> Value {
        Value
    }
}

macro_rules! impl_stub_to_logged_tuple {
    ($($name:ident),+) => {
        impl<$($name: ToLogged),+> ToLogged for ($($name,)+) {
            #[inline(always)]
            fn to_logged(&self) -> Value {
                Value
            }
        }
    };
}

impl_stub_to_logged_tuple!(A, B);
impl_stub_to_logged_tuple!(A, B, C);
impl_stub_to_logged_tuple!(A, B, C, D);
impl_stub_to_logged_tuple!(A, B, C, D, E);
impl_stub_to_logged_tuple!(A, B, C, D, E, F);
impl_stub_to_logged_tuple!(A, B, C, D, E, F, G);
impl_stub_to_logged_tuple!(A, B, C, D, E, F, G, H);
impl_stub_to_logged_tuple!(A, B, C, D, E, F, G, H, I);
impl_stub_to_logged_tuple!(A, B, C, D, E, F, G, H, I, J);
impl_stub_to_logged_tuple!(A, B, C, D, E, F, G, H, I, J, K);
impl_stub_to_logged_tuple!(A, B, C, D, E, F, G, H, I, J, K, L);

impl<T: ToLogged> ToLogged for [T] {
    #[inline(always)]
    fn to_logged(&self) -> Value {
        Value
    }
}

impl<T: ToLogged> ToLogged for Vec<T> {
    #[inline(always)]
    fn to_logged(&self) -> Value {
        Value
    }
}

impl<T: ToLogged> ToLogged for BTreeSet<T> {
    #[inline(always)]
    fn to_logged(&self) -> Value {
        Value
    }
}

impl<K: ToLogged, V: ToLogged> ToLogged for BTreeMap<K, V> {
    #[inline(always)]
    fn to_logged(&self) -> Value {
        Value
    }
}

impl<T: ToLogged, S> ToLogged for HashSet<T, S> {
    #[inline(always)]
    fn to_logged(&self) -> Value {
        Value
    }
}

impl<K: ToLogged, V: ToLogged, S> ToLogged for HashMap<K, V, S> {
    #[inline(always)]
    fn to_logged(&self) -> Value {
        Value
    }
}

/// A record: a JSON object. Inert here.
#[inline(always)]
pub fn record<N, I>(_fields: I) -> Value
where
    N: Into<String>,
    I: IntoIterator<Item = (N, Value)>,
{
    Value
}

/// A tuple: `{"#tup": […]}`. Inert here.
#[inline(always)]
pub fn tuple<I: IntoIterator<Item = Value>>(_items: I) -> Value {
    Value
}

/// Inert stand-in for one segment of an assertion path.
#[derive(Clone, Copy, Debug)]
pub struct PathSeg;

impl PathSeg {
    /// A spec variable name or record field. Inert here.
    #[inline(always)]
    pub fn ident(_name: &'static str) -> Self {
        PathSeg
    }

    /// A dynamic map key. Inert here — the live build panics on a key shape
    /// the oracle cannot resolve.
    #[inline(always)]
    pub fn value(_value: impl ToLogged) -> Self {
        PathSeg
    }

    /// The wire form of one path segment. Inert here.
    #[inline(always)]
    pub fn render(&self) -> String {
        String::new()
    }
}

/// Inert stand-in for a cross-thread test identity.
#[derive(Clone, Copy, Debug)]
pub struct TestHandle;

/// Inert stand-in for the RAII owner of a test's status report.
#[derive(Debug)]
pub struct TestGuard(());

impl TestGuard {
    /// Reports this run as failed regardless of how the test exits. Inert here.
    #[inline(always)]
    pub fn set_failed(&self) {}
}

/// Registers `name` as this thread's current test and returns the guard
/// that reports its outcome. Inert here.
#[inline(always)]
pub fn register_test(_name: &str) -> TestGuard {
    TestGuard(())
}

/// [`register_test`] with the name taken from the current thread. Inert here.
#[inline(always)]
pub fn auto_register_test() -> TestGuard {
    TestGuard(())
}

/// The test the calling thread's observations belong to. Inert here; the
/// live build panics when the oracle is not live.
#[inline(always)]
pub fn current_test() -> TestHandle {
    TestHandle
}

#[doc(hidden)]
#[inline(always)]
pub fn __register_for(_module_path: &str, _fn_name: &str) -> TestGuard {
    TestGuard(())
}

/// Mirrors the enabled impl set (`()` and `Result`) so `#[quint_oracle::test]`
/// accepts exactly the same return types in both configurations.
#[doc(hidden)]
pub trait TestOutcome {
    fn is_failure(&self) -> bool;
}

impl TestOutcome for () {
    #[inline(always)]
    fn is_failure(&self) -> bool {
        false
    }
}

impl<T, E> TestOutcome for Result<T, E> {
    #[inline(always)]
    fn is_failure(&self) -> bool {
        false
    }
}

#[doc(hidden)]
#[inline(always)]
pub fn __record_outcome<T: TestOutcome>(_ret: &T, _guard: TestGuard) {}

/// Whether instrumentation is live. Always `false` in this build — the
/// `enabled` feature is off, so no oracle code was compiled.
#[inline(always)]
pub fn enabled() -> bool {
    false
}

/// Namespace for [`Event::builder`]. Inert here.
#[derive(Debug)]
pub struct Event;

/// Inert stand-in for the observation builder; every method is a no-op.
#[derive(Debug)]
pub struct EventBuilder(());

impl Event {
    /// Starts an observation of `action` in `test`. Inert here.
    #[inline(always)]
    pub fn builder(_test: TestHandle, _action: &'static str) -> EventBuilder {
        EventBuilder(())
    }
}

impl EventBuilder {
    /// A named value the action involved. Inert here.
    #[inline(always)]
    pub fn argument(
        self,
        _name: &'static str,
        _value: impl ToLogged,
        _domain: Option<&'static str>,
    ) -> Self {
        self
    }

    /// A post-state assertion. Inert here.
    #[inline(always)]
    pub fn assert(self, _path: Vec<PathSeg>, _expected: impl ToLogged) -> Self {
        self
    }

    /// A component scope this action belongs to. Inert here.
    #[inline(always)]
    pub fn scope(self, _scope: &'static str) -> Self {
        self
    }

    /// Posts the observation. Inert here — the live build panics if the
    /// daemon rejects it.
    #[inline(always)]
    pub fn send(self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole disabled surface is callable and inert: gating for the
    /// feature-off build.
    #[test]
    fn disabled_surface_is_callable_and_inert() {
        assert!(!enabled());
        let guard = register_test("any");
        guard.set_failed();
        let _auto = auto_register_test();
        Event::builder(current_test(), "transfer")
            .argument("sender", "alice".to_logged(), Some("ACCOUNTS"))
            .argument("amount", 100u128.to_logged(), None)
            .assert(
                vec![PathSeg::ident("accounts"), PathSeg::value("alice")],
                900.to_logged(),
            )
            .scope("bank")
            .send();
        // A customer's own impl coexists with the mirrored impl set, exactly
        // as it does in the enabled build — and renders as a record, the shape
        // the constructors below exist for.
        struct Own {
            owner: String,
        }
        impl ToLogged for Own {
            fn to_logged(&self) -> Value {
                record([("owner", self.owner.to_logged())])
            }
        }
        let _ = Own {
            owner: String::from("alice"),
        }
        .to_logged();
        let _ = vec![1u8].to_logged();
        let _ = BTreeSet::from(["a"]).to_logged();
        let _ = BTreeSet::from([1.to_logged()]).to_logged();
        let _ = HashSet::from(["a"]).to_logged();
        let _ = HashMap::from([("a", 1)]).to_logged();
        let _ = tuple([1.to_logged(), "a".to_logged()]);
        let _ = guard;
    }
}
