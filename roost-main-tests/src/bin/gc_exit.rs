//! Leaves an unreachable cycle behind when `main` returns. Its objects print when they are
//! finalized; the test checks that this happens after `main` returned and before the
//! `gc_stats` line, i.e. in the shell's final collection.

use roost::gc::{Finalize, Trace, unsafe_empty_trace};
use roost::prelude::*;

/// A field that prints when its object is finalized.
#[derive(Clone, Default)]
struct Loud;

impl Finalize for Loud {
    fn finalize(&self) {
        println!("finalized a Node");
    }
}

// SAFETY: no GC pointers inside.
unsafe impl Trace for Loud {
    unsafe_empty_trace!();
}

class! {
    class Node {
        loud: Loud;
        next: Option<Ref<Node>>;
        public fn link(next: Ref<Node>) { this.next = Some(next); }
    }
}

#[roost::main(gc_threshold = "1MB", gc_stats)]
fn main() {
    let a = new!(Node());
    let b = new!(Node());
    a.link(b.clone());
    b.link(a); // a <-> b: reference counting alone never frees this
    println!("main returns");
}
