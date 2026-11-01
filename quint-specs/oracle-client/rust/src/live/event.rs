//! The observation builder: the un-ordered escape hatch the `log!` macro
//! lowers into, and the wire rendering of one logged event.

use itf::Value;

use super::registry::TestHandle;
use super::transport;
use super::value::ToLogged;

/// One segment of an assertion path: from a spec variable name through record
/// fields and map keys down to the value being asserted on.
#[derive(Debug, Clone)]
pub enum PathSeg {
    /// A variable name or record field.
    Ident(&'static str),
    /// A dynamic map key.
    Value(Value),
}

impl PathSeg {
    #[inline]
    pub fn ident(name: &'static str) -> Self {
        PathSeg::Ident(name)
    }

    /// A dynamic value. Panics unless the value renders as a string or an `i64`.
    pub fn value(value: impl ToLogged) -> Self {
        let value = value.to_logged();
        match &value {
            Value::String(_) | Value::Number(_) => PathSeg::Value(value),
            other => panic!(
                "an assertion path segment must be a string or an \
                 i64 — the daemon resolves no other key shape; got {other:?}"
            ),
        }
    }

    /// The wire form: the daemon takes paths as strings, so strings render raw
    /// (unquoted) and integers as decimal.
    pub fn render(&self) -> String {
        match self {
            PathSeg::Ident(name) => (*name).to_string(),
            PathSeg::Value(Value::String(s)) => s.clone(),
            PathSeg::Value(Value::Number(n)) => n.to_string(),
            PathSeg::Value(other) => unreachable!("unsupported path segment: {other:?}"),
        }
    }
}

/// Namespace for [`Event::builder`]; an event only exists on the wire.
pub struct Event;

impl Event {
    /// Start an observation of `action` in `test`.
    pub fn builder(test: TestHandle, action: &'static str) -> EventBuilder {
        EventBuilder {
            test,
            action,
            scopes: Vec::new(),
            arguments: Vec::new(),
            assertions: Vec::new(),
        }
    }
}

/// Accumulates one observation's parts in any order; [`EventBuilder::send`]
/// posts it. Building is cheap but not free — gate call sites on
/// [`crate::enabled()`] when the values are expensive to render (the `log!`
/// macro does).
pub struct EventBuilder {
    test: TestHandle,
    action: &'static str,
    scopes: Vec<&'static str>,
    arguments: Vec<serde_json::Value>,
    assertions: Vec<serde_json::Value>,
}

impl EventBuilder {
    /// A nondeterministic value that is part of the action's execution —
    /// typically passed as arguments to the action. `domain` describes which
    /// spec constant this value should be added to so that trace validation is
    /// not limited to hardcoded constants in the specification.
    pub fn argument(
        mut self,
        name: &'static str,
        value: impl ToLogged,
        domain: Option<&'static str>,
    ) -> Self {
        let mut arg = serde_json::json!({ "name": name, "value": value.to_logged() });
        if let Some(domain) = domain {
            arg["domain"] = serde_json::json!(domain);
        }
        self.arguments.push(arg);
        self
    }

    /// A post-state assertion: after this action, the spec value at `path` must
    /// be `expected`.
    pub fn assert(mut self, path: Vec<PathSeg>, expected: impl ToLogged) -> Self {
        let path: Vec<String> = path.iter().map(PathSeg::render).collect();
        self.assertions
            .push(serde_json::json!({ "path": path, "value": expected.to_logged() }));
        self
    }

    /// A component scope this action belongs to; call once per scope. The
    /// daemon requires at least one — a scopeless event is recorded as dropped,
    /// not validated.
    pub fn scope(mut self, scope: &'static str) -> Self {
        self.scopes.push(scope);
        self
    }

    /// Post the observation to the daemon.
    ///
    /// Panics if the daemon *rejects* it — a refused event means the
    /// instrumentation is wrong (a missing scope invalidates the whole run).
    pub fn send(self) {
        let body = serde_json::json!({
            "action": self.action,
            "scopes": self.scopes,
            "arguments": self.arguments,
            "assertions": self.assertions,
        });
        if let Err(error) = transport::post_event(self.test.name(), &body) {
            panic!("{error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_segments_render_to_the_daemon_string_forms() {
        assert_eq!(PathSeg::ident("accounts").render(), "accounts");
        assert_eq!(PathSeg::value("alice").render(), "alice");
        assert_eq!(PathSeg::value(42).render(), "42");
        assert_eq!(PathSeg::value(-7i64).render(), "-7");
    }

    #[test]
    #[should_panic(expected = "must be a string or an i64")]
    fn bool_path_segments_are_rejected() {
        PathSeg::value(true);
    }

    #[test]
    #[should_panic(expected = "must be a string or an i64")]
    fn collection_path_segments_are_rejected() {
        PathSeg::value(vec![1, 2]);
    }

    /// Beyond `i64` the value takes the `#bigint` form, which the daemon's
    /// `i64` key parse cannot match.
    #[test]
    #[should_panic(expected = "must be a string or an i64")]
    fn wide_int_path_segments_are_rejected() {
        PathSeg::value(u64::MAX);
    }
}
