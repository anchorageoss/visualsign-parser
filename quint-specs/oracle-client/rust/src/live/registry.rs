//! Which test an observation belongs to, and how a test's outcome is reported.
//!
//! Lifecycle, lazy mode: the first observation on a test's thread installs the
//! chained panic hook (once), registers the test from the thread name (in the
//! process-wide open registry and this thread's `CURRENT`), events post as
//! they happen, and the TLS destructor at thread exit consults the panic
//! registry and PATCHes the run's status. Explicit mode ([`register_test`]) is
//! the same with the returned guard's `Drop` doing the PATCH — correct even
//! without the hook, since it runs *during* unwinding.
//!
//! Spawned/worker threads never register anything: each of their events
//! resolves through the sole-open-test fallback, and panics when that is
//! ambiguous — never mis-attributes.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::sync::{Arc, Mutex, Once, OnceLock};
use std::thread::{self, ThreadId};

use super::transport;

/// Cheap cross-thread test identity (clone it into tokio tasks, spawned
/// threads); [`crate::Event::builder`] takes one.
#[derive(Clone, Debug)]
pub struct TestHandle(Arc<TestShared>);

#[derive(Debug)]
struct TestShared {
    name: String,
}

impl TestHandle {
    fn new(name: &str) -> Self {
        TestHandle(Arc::new(TestShared {
            name: name.to_string(),
        }))
    }

    pub(crate) fn name(&self) -> &str {
        &self.0.name
    }

    /// Identity, not name equality: two runs of the same test name are
    /// distinct handles.
    fn same(&self, other: &TestHandle) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// RAII owner of the test's status report: `Drop` signals test completion to
/// the quint oracle daemon; inert when the oracle is disabled.
#[derive(Debug)]
pub struct TestGuard {
    handle: Option<TestHandle>,
    thread: ThreadId,
    failed: Cell<bool>,
}

impl TestGuard {
    fn inert() -> Self {
        TestGuard {
            handle: None,
            thread: thread::current().id(),
            failed: Cell::new(false),
        }
    }

    /// Report this run as failed regardless of how the test exits.
    pub fn set_failed(&self) {
        self.failed.set(true);
    }
}

impl Drop for TestGuard {
    fn drop(&mut self) {
        let Some(handle) = self.handle.take() else {
            return;
        };
        let failed = self.failed.get() || thread::panicking() || panicked_take(self.thread);
        // Leave the open registry FIRST, so a late worker event panics rather
        // than attach to a finalized run.
        open_remove(&handle);
        // Discarded deliberately: this runs while unwinding (`TestGuard`) or
        // from a TLS destructor (`CurrentTest`), where a panic aborts the
        // process instead of failing one test.
        let _ = transport::patch_status(
            handle.name(),
            if failed {
                transport::RunStatus::Failed
            } else {
                transport::RunStatus::Ok
            },
        );
        // try_with: a guard may drop during TLS teardown, after CURRENT is
        // gone. Clear only our own registration — a later guard on this
        // thread owns the slot by then.
        let _ = CURRENT.try_with(|current| {
            let mut current = current.borrow_mut();
            if current.as_ref().is_some_and(|cur| cur.handle.same(&handle)) {
                *current = None;
            }
        });
    }
}

/// What `log!` resolves against on this thread.
struct CurrentTest {
    handle: TestHandle,
    /// True only for lazy registration: the TLS destructor owns the PATCH.
    /// Explicit registration reports through the guard instead.
    owns_patch: bool,
    /// Captured at registration: `thread::current()` may itself panic inside
    /// the TLS destructor.
    thread: ThreadId,
}

impl Drop for CurrentTest {
    fn drop(&mut self) {
        if !self.owns_patch {
            return;
        }
        // `thread::panicking()` is false here (libtest caught the panic before
        // the thread exits), so the outcome comes from the panic registry.
        // take()n, so a pooled thread never inherits a stale failure.
        let failed = panicked_take(self.thread);
        open_remove(&self.handle);
        // Discarded deliberately: this runs while unwinding (`TestGuard`) or
        // from a TLS destructor (`CurrentTest`), where a panic aborts the
        // process instead of failing one test.
        let _ = transport::patch_status(
            self.handle.name(),
            if failed {
                transport::RunStatus::Failed
            } else {
                transport::RunStatus::Ok
            },
        );
    }
}

thread_local! {
    static CURRENT: RefCell<Option<CurrentTest>> = const { RefCell::new(None) };
}

static HOOK: Once = Once::new();
static PANICKED: OnceLock<Mutex<HashSet<ThreadId>>> = OnceLock::new();
/// Every open (registered, not yet reported) test, process-wide: the
/// sole-open-test fallback resolves against this.
static OPEN: OnceLock<Mutex<Vec<TestHandle>>> = OnceLock::new();

/// Register `name` as this thread's current test and return the guard that
/// reports its outcome. Takes precedence over lazy registration; keep the
/// guard alive for the whole test (`let _guard = …`).
///
/// Registering a name the oracle has already seen completed opens a new run —
/// each guard reports exactly one run.
pub fn register_test(name: &str) -> TestGuard {
    if !super::enabled() {
        return TestGuard::inert();
    }
    install_panic_hook_once();
    let handle = TestHandle::new(name);
    open_insert(&handle);
    let thread = thread::current().id();
    CURRENT.with(|current| {
        *current.borrow_mut() = Some(CurrentTest {
            handle: handle.clone(),
            owns_patch: false,
            thread,
        });
    });
    TestGuard {
        handle: Some(handle),
        thread,
        failed: Cell::new(false),
    }
}

/// [`register_test`] with the name taken from the current thread's name (the
/// Rust test harness names each test's thread after the test).
///
/// Panics when the thread name cannot name a test — the main thread, unnamed
/// threads, runtime worker threads. No-op (inert guard) when the oracle is
/// disabled.
pub fn auto_register_test() -> TestGuard {
    if !super::enabled() {
        return TestGuard::inert();
    }
    match usable_thread_name() {
        Some(name) => register_test(&name),
        None => panic!(
            "cannot infer a test name from this thread; \
             use register_test(..) or #[quint_oracle::test]"
        ),
    }
}

/// The test the calling thread's observations belong to. Resolution order:
///
/// 1. this thread's registration (explicit guard or earlier lazy one);
/// 2. a usable thread name — lazily registers THIS thread's own test;
/// 3. no usable name (spawned/worker thread): the sole open test iff exactly
///    one is open;
/// 4. otherwise panic: pre-register with
///    `#[quint_oracle::test]`/[`register_test`], or run with
///    `--test-threads=1`.
///
/// Panics when the oracle is not live ([`crate::enabled`] is false) — unlike
/// [`register_test`], which stays inert there. Every call site must be guarded;
/// the `log!` macro is.
pub fn current_test() -> TestHandle {
    assert!(
        super::enabled(),
        "current_test() needs a live oracle — guard call sites \
         with quint_oracle::enabled(); the log! macro already does"
    );
    CURRENT.with(|current| {
        if let Some(cur) = &*current.borrow() {
            return cur.handle.clone();
        }
        // Completes BEFORE this observation returns, so later panics are
        // always seen.
        install_panic_hook_once();
        if let Some(name) = usable_thread_name() {
            let handle = TestHandle::new(&name);
            open_insert(&handle);
            *current.borrow_mut() = Some(CurrentTest {
                handle: handle.clone(),
                owns_patch: true,
                thread: thread::current().id(),
            });
            return handle;
        }
        match open_sole() {
            Ok(handle) => handle,
            Err(open_count) => panic!(
                "cannot attribute this event: {open_count} tests \
                 are open and this thread has no test; pre-register with \
                 #[quint_oracle::test] or run with --test-threads=1"
            ),
        }
    })
}

/// `#[quint_oracle::test]`'s registration entry point: registers the fn's
/// libtest name before the body runs. Hidden — the attribute expansion is its
/// only caller.
#[doc(hidden)]
pub fn __register_for(module_path: &str, fn_name: &str) -> TestGuard {
    register_test(&derive_test_name(module_path, fn_name))
}

/// What `#[quint_oracle::test]` reads a failure from: `()` never fails,
/// `Result` fails on `Err`. Panics bypass this and report through the guard's
/// `Drop` instead.
#[doc(hidden)]
pub trait TestOutcome {
    fn is_failure(&self) -> bool;
}

impl TestOutcome for () {
    fn is_failure(&self) -> bool {
        false
    }
}

impl<T, E> TestOutcome for Result<T, E> {
    fn is_failure(&self) -> bool {
        self.is_err()
    }
}

/// `#[quint_oracle::test]`'s reporting exit point: marks the guard failed if
/// the returned value is a failure, then drops it — which PATCHes the run.
#[doc(hidden)]
pub fn __record_outcome<T: TestOutcome>(ret: &T, guard: TestGuard) {
    if ret.is_failure() {
        guard.set_failed();
    }
}

/// The libtest name of a test fn: `module_path!()` minus its leading crate
/// segment, joined with the fn name — `tests::my_test` for unit tests,
/// `my_test` for integration tests at the test crate's root.
pub(crate) fn derive_test_name(module_path: &str, fn_name: &str) -> String {
    match module_path.split_once("::") {
        Some((_, rest)) => format!("{rest}::{fn_name}"),
        None => fn_name.to_string(),
    }
}

/// The current thread's name, iff it can name a test: `None` for unnamed
/// threads, the main thread, and runtime pool threads.
fn usable_thread_name() -> Option<String> {
    let thread = thread::current();
    let name = thread.name().filter(|name| names_a_test(name))?;
    Some(name.to_string())
}

/// Whether a thread name could be a libtest test name: a non-"main"
/// `::`-path of Rust identifiers. Runtime pool threads never qualify — their
/// names ("tokio-rt-worker", "tokio-runtime-worker", "async-std/runtime")
/// contain characters an identifier path cannot.
fn names_a_test(name: &str) -> bool {
    if name == "main" {
        return false;
    }
    !name.is_empty()
        && name.split("::").all(|segment| {
            let mut chars = segment.chars();
            chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
                && chars.all(|c| c.is_alphanumeric() || c == '_')
        })
}

fn install_panic_hook_once() {
    HOOK.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            record_panicking_thread();
            // Chained, so libtest's output capture (and any user hook)
            // is preserved.
            prev(info);
        }));
    });
}

/// Lock recovery everywhere: a panic while one of these mutexes is held (only
/// possible via an allocation failure) must not poison outcome reporting for
/// every other test.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn record_panicking_thread() {
    lock(PANICKED.get_or_init(Default::default)).insert(thread::current().id());
}

fn panicked_take(thread: ThreadId) -> bool {
    lock(PANICKED.get_or_init(Default::default)).remove(&thread)
}

fn open_insert(handle: &TestHandle) {
    lock(OPEN.get_or_init(Default::default)).push(handle.clone());
}

fn open_remove(handle: &TestHandle) {
    lock(OPEN.get_or_init(Default::default)).retain(|open| !open.same(handle));
}

/// The sole open test, or how many are open when that is not exactly one.
fn open_sole() -> Result<TestHandle, usize> {
    let open = lock(OPEN.get_or_init(Default::default));
    match open.as_slice() {
        [sole] => Ok(sole.clone()),
        others => Err(others.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn libtest_names_derive_from_module_path_and_fn() {
        // Unit test in a nested module of crate `mycrate`.
        assert_eq!(
            derive_test_name("mycrate::api::tests", "transfers_settle"),
            "api::tests::transfers_settle"
        );
        // Integration test at the test crate's root: module_path!() is just
        // the crate segment, which is dropped whole.
        assert_eq!(
            derive_test_name("e2e", "transfers_settle"),
            "transfers_settle"
        );
    }

    #[test]
    fn thread_names_that_cannot_be_tests_are_unusable() {
        assert!(names_a_test("my_test"));
        assert!(names_a_test("tests::my_test"));
        assert!(names_a_test("api::tests::transfers_settle"));
        assert!(!names_a_test(""));
        assert!(!names_a_test("main"));
        assert!(!names_a_test("tokio-rt-worker")); // tokio ≥ 1.52
        assert!(!names_a_test("tokio-runtime-worker")); // older tokio
        assert!(!names_a_test("async-std/runtime"));
        assert!(!names_a_test("some worker"));
        assert!(!names_a_test("tests::")); // empty trailing segment
    }

    /// Feature on but no `QUINT_ORACLE_URL`: the whole surface must be inert
    /// (no open-registry entries, no panics from `current_test`), because
    /// customers run instrumented test builds outside Studio too.
    #[test]
    fn without_a_daemon_url_the_enabled_build_is_inert() {
        std::env::remove_var("QUINT_ORACLE_URL");
        assert!(!crate::enabled());
        let guard = register_test("some::test");
        assert_eq!(
            open_sole().err(),
            Some(0),
            "an inert registration must not join the open registry"
        );
        guard.set_failed();
        drop(guard);
        drop(auto_register_test()); // must not panic on the unusable name
    }

    /// The one item that is NOT inert without a daemon: resolving the current
    /// test is a programming error there (an unguarded call site), not a no-op.
    #[test]
    #[should_panic(expected = "needs a live oracle")]
    fn current_test_without_a_daemon_url_panics() {
        std::env::remove_var("QUINT_ORACLE_URL");
        let _ = current_test();
    }
}
