//! The program shell behind [`#[roost::main]`](crate::main): a `main` thread with a big
//! stack, GC settings, a final collection, Java-style reports for uncaught roost exceptions
//! and for an `Err` returned from `main`.
//!
//! Use it through the attribute. [`run`] is public so the shell can also be driven by hand
//! (for example from a test harness), and [`stats`] works with or without it.

use std::any::type_name;
use std::cell::{Cell, RefCell};
use std::fmt;
use std::io::Write;
use std::panic::{self, Location, PanicHookInfo};
use std::sync::Once;

/// What `#[roost::main(..)]` configures. Every field has a default; see [`Config::default`].
#[derive(Clone, Debug)]
pub struct Config {
    /// Stack size of the thread `main` runs on, in bytes. Default: 64 MiB. Deep object
    /// graphs need it: the collector's mark phase recurses along references.
    pub stack: usize,
    /// Bytes allocated on the GC heap before the first automatic collection. `None` keeps
    /// the collector's own default (it grows the threshold as the heap grows).
    pub gc_threshold: Option<usize>,
    /// Print allocation / collection statistics to stderr on exit.
    pub gc_stats: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            stack: 64 << 20,
            gc_threshold: None,
            gc_stats: false,
        }
    }
}

/// Runs `main` on a new thread named `main` configured by `config`, then exits the process.
///
/// `main` returns the exit code (the attribute turns `Ok(())` / `()` into 0 and a reported
/// `Err` into 1). After it returns, on the same thread: one full collection, then the
/// statistics if asked for. A roost exception (see [`throw`]) that is not caught ends the
/// process with exit code 1 after printing `Exception in thread "main" ...`; any other
/// panic is reported by the previous panic hook as usual and exits with 101, like a plain
/// Rust `main`.
pub fn run(config: Config, main: impl FnOnce() -> i32 + Send + 'static) -> ! {
    install_panic_hook();
    let Config {
        stack,
        gc_threshold,
        gc_stats,
    } = config;
    let thread = std::thread::Builder::new()
        .name("main".into())
        .stack_size(stack)
        .spawn(move || {
            if let Some(threshold) = gc_threshold {
                ::gc::configure(|c| c.threshold = threshold);
            }
            // A panic has been reported by the hook by now; skip the shutdown work.
            let code = match panic::catch_unwind(panic::AssertUnwindSafe(main)) {
                Ok(code) => code,
                Err(_) if REPORTED.with(Cell::get) => return 1,
                Err(_) => return 101,
            };
            let before = stats();
            crate::gc::collect();
            if gc_stats {
                print_stats(&before, &stats());
            }
            code
        })
        .unwrap_or_else(|e| {
            eprintln!("roost: cannot start the main thread ({stack} byte stack): {e}");
            std::process::exit(1)
        });
    // `Err` only if the shell itself panicked (e.g. in `collect`): already reported.
    let code = thread.join().unwrap_or(101);
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    std::process::exit(code)
}

// ---------------------------------------------------------------------------------------
// Statistics

thread_local! {
    static OBJECTS_ALLOCATED: Cell<u64> = const { Cell::new(0) };
}

/// Counts one object allocated through [`Ref::new`](crate::Ref::new).
#[inline]
pub(crate) fn count_allocation() {
    OBJECTS_ALLOCATED.with(|n| n.set(n.get() + 1));
}

/// The current thread's collector statistics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GcStats {
    /// Objects allocated with [`Ref::new`](crate::Ref::new) (every `new` of a class) so far.
    pub objects_allocated: u64,
    /// Bytes of GC heap in use right now (allocated and not yet freed).
    pub live_bytes: usize,
    /// Collections run so far, automatic and explicit.
    pub collections: usize,
}

/// Statistics for the current thread's GC heap.
pub fn stats() -> GcStats {
    let s = ::gc::stats();
    GcStats {
        objects_allocated: OBJECTS_ALLOCATED.with(Cell::get),
        live_bytes: s.bytes_allocated,
        collections: s.collections_performed,
    }
}

fn print_stats(before: &GcStats, after: &GcStats) {
    let _ = std::io::stdout().flush();
    eprintln!(
        "[roost gc] objects allocated: {}, collections: {} (1 at exit), heap at exit: {} bytes, \
         freed by the final collection: {} bytes, left: {} bytes",
        after.objects_allocated,
        after.collections,
        before.live_bytes,
        before.live_bytes.saturating_sub(after.live_bytes),
        after.live_bytes,
    );
}

// ---------------------------------------------------------------------------------------
// Exceptions

/// The kind of a roost exception, printed as `roost.<Kind>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExceptionKind {
    /// A field was accessed while another access to it was in progress (only possible
    /// when user code runs *during* a field access, e.g. a `Clone` impl that calls back
    /// into the object).
    BorrowConflict,
}

impl fmt::Display for ExceptionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ExceptionKind::BorrowConflict => "BorrowConflict",
        })
    }
}

/// A runtime error raised by roost itself, with the class / field it is about.
#[derive(Clone, Debug)]
pub struct Exception {
    pub kind: ExceptionKind,
    pub message: String,
}

impl fmt::Display for Exception {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "roost.{}: {}", self.kind, self.message)
    }
}

thread_local! {
    /// The exception being thrown on this thread, between [`throw`] and the panic hook.
    static PENDING: RefCell<Option<String>> = const { RefCell::new(None) };
    /// Whether the hook printed this thread's last panic as a roost exception.
    static REPORTED: Cell<bool> = const { Cell::new(false) };
}

/// Throws `e`: panics with the message `roost.Kind: message`.
///
/// Without `#[roost::main]` that is an ordinary panic whose message names the class and
/// field. Under `#[roost::main]` it is printed like an uncaught Java exception:
///
/// ```text
/// Exception in thread "main" roost.BorrowConflict: field Animal.name ...
///     at src/main.rs:12:9
/// ```
#[track_caller]
pub fn throw(e: Exception) -> ! {
    let msg = e.to_string();
    PENDING.with(|p| *p.borrow_mut() = Some(msg.clone()));
    panic!("{msg}")
}

fn payload_str(payload: &(dyn std::any::Any + Send)) -> Option<&str> {
    payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&'static str>().copied())
}

fn install_panic_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info: &PanicHookInfo<'_>| {
            let pending = PENDING.with(|p| p.borrow_mut().take());
            let msg = payload_str(info.payload());
            match (pending, msg) {
                (Some(p), Some(m)) if p == m => {
                    REPORTED.with(|r| r.set(true));
                    report_exception(m, info.location());
                }
                _ => previous(info),
            }
        }));
    });
}

fn report_exception(msg: &str, location: Option<&Location<'_>>) {
    let thread = std::thread::current();
    let name = thread.name().unwrap_or("<unnamed>");
    let mut out = String::new();
    out.push_str(&format!("Exception in thread \"{name}\" {msg}\n"));
    if let Some(l) = location {
        out.push_str(&format!("\tat {}:{}:{}\n", l.file(), l.line(), l.column()));
    }
    let bt = std::backtrace::Backtrace::capture();
    if bt.status() == std::backtrace::BacktraceStatus::Captured {
        out.push_str(&format!("{bt}\n"));
    } else {
        out.push_str("\t(run with `RUST_BACKTRACE=1` for the full stack)\n");
    }
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().write_all(out.as_bytes());
}

// ---------------------------------------------------------------------------------------
// `Err` from main

/// `main`'s error, reported the most informative way its type allows: as an
/// [`Error`](std::error::Error) (with its `Caused by:` chain), else with `Display`, else
/// with `Debug`. The attribute calls `(&&&&Report(&err)).roost_report()`; which impl applies
/// is decided by autoref at the (concrete) call site.
#[doc(hidden)]
pub struct Report<'a, E>(pub &'a E);

#[doc(hidden)]
pub trait ReportError {
    fn roost_report(&self);
}
#[doc(hidden)]
pub trait ReportDisplay {
    fn roost_report(&self);
}
#[doc(hidden)]
pub trait ReportDebug {
    fn roost_report(&self);
}

/// What `fn main` may return under `#[roost::main]` (besides `()`).
#[doc(hidden)]
#[diagnostic::on_unimplemented(
    message = "`#[roost::main]` needs `fn main()` or `fn main() -> Result<(), E>` with `E: Debug`, not `-> {Self}`",
    label = "not `Result<(), E>`"
)]
pub trait MainResult {}

impl<E: fmt::Debug> MainResult for Result<(), E> {}

/// Compile-time check of `main`'s return type, for a readable error.
#[doc(hidden)]
pub fn assert_main_result<T: MainResult>() {}

#[doc(hidden)]
pub trait ReportBoxedError {
    fn roost_report(&self);
}

// `Box<dyn Error>`, `anyhow::Error`, ...: not `Error` themselves, but point to one.
impl<E, D> ReportBoxedError for &&&Report<'_, E>
where
    E: std::ops::Deref<Target = D>,
    D: std::error::Error + ?Sized,
{
    fn roost_report(&self) {
        emit(&with_causes(header::<E>(&self.0.to_string()), &**self.0));
    }
}

impl<E: std::error::Error> ReportError for &&Report<'_, E> {
    fn roost_report(&self) {
        emit(&with_causes(header::<E>(&self.0.to_string()), self.0));
    }
}

/// Java's `Caused by:` lines, one per `source()`.
fn with_causes(mut text: String, e: &(impl std::error::Error + ?Sized)) -> String {
    let mut source = e.source();
    while let Some(s) = source {
        text.push_str(&format!("Caused by: {s}\n"));
        source = s.source();
    }
    text
}

impl<E: fmt::Display> ReportDisplay for &Report<'_, E> {
    fn roost_report(&self) {
        emit(&header::<E>(&self.0.to_string()));
    }
}

impl<E: fmt::Debug> ReportDebug for Report<'_, E> {
    fn roost_report(&self) {
        emit(&header::<E>(&format!("{:?}", self.0)));
    }
}

fn header<E>(msg: &str) -> String {
    format!(
        "Exception in thread \"{}\" {}: {msg}\n",
        std::thread::current().name().unwrap_or("<unnamed>"),
        short_type_name(type_name::<E>())
    )
}

fn emit(text: &str) {
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().write_all(text.as_bytes());
}

/// `alloc::boxed::Box<dyn core::error::Error>` -> `Box<dyn Error>`: drops module paths.
fn short_type_name(full: &str) -> String {
    let mut out = String::new();
    let mut segment = String::new();
    let mut chars = full.chars().peekable();
    while let Some(c) = chars.next() {
        if c == ':' && chars.peek() == Some(&':') {
            chars.next();
            segment.clear();
        } else if c.is_alphanumeric() || c == '_' {
            segment.push(c);
        } else {
            out.push_str(&segment);
            segment.clear();
            out.push(c);
        }
    }
    out.push_str(&segment);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_type_names() {
        assert_eq!(short_type_name("std::io::error::Error"), "Error");
        assert_eq!(
            short_type_name("alloc::boxed::Box<dyn core::error::Error + core::marker::Send>"),
            "Box<dyn Error + Send>"
        );
        assert_eq!(short_type_name("my_app::MyError"), "MyError");
        assert_eq!(short_type_name("(i32, &str)"), "(i32, &str)");
    }

    #[test]
    fn exception_message_names_kind() {
        let e = Exception {
            kind: ExceptionKind::BorrowConflict,
            message: "field Animal.name".into(),
        };
        assert_eq!(e.to_string(), "roost.BorrowConflict: field Animal.name");
    }

    #[test]
    fn field_conflicts_panic_with_class_and_field() {
        use crate::__private::{field, get, set};
        use crate::gc::GcCell;
        let cell = GcCell::new(1_i32);
        let reading = cell.borrow();
        let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            set(field(&cell, "Animal", "legs"), 2)
        }))
        .expect_err("a write during a read conflicts");
        let msg = payload_str(&*err).unwrap();
        assert!(
            msg.starts_with("roost.BorrowConflict: field Animal.legs cannot be written"),
            "{msg}"
        );
        drop(reading);
        let writing = cell.borrow_mut();
        let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            get(field(&cell, "Animal", "legs"))
        }))
        .expect_err("a read during a write conflicts");
        let msg = payload_str(&*err).unwrap();
        assert!(
            msg.starts_with("roost.BorrowConflict: field Animal.legs cannot be read"),
            "{msg}"
        );
        drop(writing);
        // Both were recorded for the hook, which isn't installed here; nothing is left over.
        PENDING.with(|p| p.borrow_mut().take());
        assert_eq!(get(field(&cell, "Animal", "legs")), 1);
    }

    #[test]
    fn stats_count_allocations_and_collections() {
        let before = stats();
        let _r = crate::Ref::new(1_i32);
        crate::gc::collect();
        let after = stats();
        assert_eq!(after.objects_allocated, before.objects_allocated + 1);
        assert!(after.collections > before.collections);
        assert!(after.live_bytes > 0);
    }
}
