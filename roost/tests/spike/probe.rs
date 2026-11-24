//! Thread-local counters used to prove that objects are actually reclaimed.
//!
//! The gc crate is a thread-local mark-sweep collector and the test harness runs every test
//! on its own thread, so each test sees its own heap and its own counters.

use std::cell::Cell;

use gc::{Finalize, Trace};

thread_local! {
    static DROPPED: Cell<usize> = const { Cell::new(0) };
    static FINALIZED: Cell<usize> = const { Cell::new(0) };
}

/// Number of `DropProbe`s dropped on this thread (i.e. objects whose memory was freed).
pub fn dropped() -> usize {
    DROPPED.with(Cell::get)
}

/// Number of `Finalize::finalize` calls recorded on this thread.
pub fn finalized() -> usize {
    FINALIZED.with(Cell::get)
}

pub(crate) fn record_finalize() {
    FINALIZED.with(|c| c.set(c.get() + 1));
}

/// Embedded in every animal. Its `Drop` runs when the collector frees the owning object.
///
/// (`#[derive(Trace)]` already generates a `Drop` impl for the class structs, so a class
/// cannot have its own `Drop`; a probe field is the way to observe the free.)
#[derive(Default)]
pub struct DropProbe;

impl Drop for DropProbe {
    fn drop(&mut self) {
        DROPPED.with(|c| c.set(c.get() + 1));
    }
}

impl Finalize for DropProbe {}

// SAFETY: contains no `Gc` pointers.
unsafe impl Trace for DropProbe {
    gc::unsafe_empty_trace!();
}
