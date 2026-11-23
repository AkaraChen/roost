//! The collector, as seen from roost: re-exports from the vendored `gc` crate, an explicit
//! [`collect`], and [`Counter`] / [`Probe`] for tests that check objects really are
//! finalized and freed.
//!
//! The collector is thread-local, and the test harness runs each test on its own thread,
//! so every test has its own heap.

use std::cell::Cell;
use std::rc::Rc;

pub use ::gc::{
    Finalize, Gc, GcCell, GcCellRef, GcCellRefMut, Trace, custom_trace, finalizer_safe,
    unsafe_empty_trace,
};

/// Runs a full collection on the current thread now: every object unreachable from a
/// root is finalized, then freed. Java's `System.gc()`, except that it really runs.
/// The collector also runs by itself as the heap grows; this is for tests and for
/// programs that want to free a large structure at a known point.
///
/// # Panics
///
/// If called while a collection is already running (e.g. from a `finalize`).
pub fn collect() {
    ::gc::force_collect();
}

#[derive(Default)]
struct Counts {
    finalized: Cell<usize>,
    dropped: Cell<usize>,
}

/// Counts how many of its [`Probe`]s were finalized and dropped.
///
/// Put a probe in a field of each object under test; `Counter` itself is a plain `Rc`
/// handle, so it never keeps the objects alive.
///
/// ```
/// use roost::gc::{self, Counter, Finalize, Probe, Trace};
///
/// #[derive(Trace, Finalize)]
/// struct Node {
///     probe: Probe,
///     next: gc::GcCell<Option<gc::Gc<Node>>>,
/// }
///
/// let counter = Counter::new();
/// let a = gc::Gc::new(Node { probe: counter.probe(), next: gc::GcCell::new(None) });
/// *a.next.borrow_mut() = Some(a.clone()); // a -> a
/// drop(a);
/// assert_eq!(counter.dropped(), 0);
/// gc::collect();
/// assert_eq!((counter.finalized(), counter.dropped()), (1, 1));
/// ```
#[derive(Clone, Default)]
pub struct Counter(Rc<Counts>);

impl Counter {
    /// A counter with no probes yet.
    pub fn new() -> Counter {
        Counter::default()
    }

    /// A new probe reporting to this counter.
    pub fn probe(&self) -> Probe {
        Probe(self.0.clone())
    }

    /// How many probes have been finalized. Finalization runs when the collector finds the
    /// owning object unreachable, before freeing it. It needs the owner's `finalize_glue`
    /// to reach the probe, which `#[derive(Trace)]` does for every (non-ignored) field.
    pub fn finalized(&self) -> usize {
        self.0.finalized.get()
    }

    /// How many probes have been dropped, i.e. how many owning objects were freed.
    pub fn dropped(&self) -> usize {
        self.0.dropped.get()
    }
}

/// A field that reports to its [`Counter`] when the owning object is finalized and when
/// it is dropped. Holds no GC pointers.
pub struct Probe(Rc<Counts>);

impl Finalize for Probe {
    fn finalize(&self) {
        self.0.finalized.set(self.0.finalized.get() + 1);
    }
}

// SAFETY: `Probe` contains no `Gc` pointers.
unsafe impl Trace for Probe {
    unsafe_empty_trace!();
}

impl Drop for Probe {
    fn drop(&mut self) {
        self.0.dropped.set(self.0.dropped.get() + 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Trace, Finalize)]
    struct Pair {
        probe: Probe,
        other: GcCell<Option<Gc<Pair>>>,
    }

    fn pair(counter: &Counter) -> Gc<Pair> {
        Gc::new(Pair {
            probe: counter.probe(),
            other: GcCell::new(None),
        })
    }

    #[test]
    fn collect_frees_an_unreachable_cycle() {
        let counter = Counter::new();
        let a = pair(&counter);
        let b = pair(&counter);
        *a.other.borrow_mut() = Some(b.clone());
        *b.other.borrow_mut() = Some(a.clone());
        drop((a, b));

        // Reference counting alone never frees a cycle.
        assert_eq!((counter.finalized(), counter.dropped()), (0, 0));
        collect();
        assert_eq!((counter.finalized(), counter.dropped()), (2, 2));
    }

    #[test]
    fn collect_keeps_rooted_objects() {
        let counter = Counter::new();
        let a = pair(&counter);
        *a.other.borrow_mut() = Some(pair(&counter)); // reachable only through `a`
        collect();
        assert_eq!(counter.dropped(), 0);
        drop(a);
        collect();
        assert_eq!(counter.dropped(), 2);
    }

    #[test]
    fn counters_are_independent() {
        let (c1, c2) = (Counter::new(), Counter::new());
        drop(pair(&c1));
        collect();
        assert_eq!((c1.dropped(), c2.dropped()), (1, 0));
    }
}
