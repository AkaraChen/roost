//! A borrow conflict on a static field, inherited by the class that touches it: like
//! `borrow_conflict.rs`, `Name::clone` writes the field while `Dog::count` is being copied
//! out. The report names the declaring class, `Animal.count`, like an instance field.

use std::cell::RefCell;

use roost::gc::{Finalize, Trace, unsafe_empty_trace};
use roost::prelude::*;

thread_local! {
    static ON_CLONE: RefCell<Option<Box<dyn Fn()>>> = RefCell::new(None);
}

#[derive(Default)]
struct Name;

impl Clone for Name {
    fn clone(&self) -> Self {
        ON_CLONE.with(|f| {
            if let Some(f) = f.borrow().as_ref() {
                f();
            }
        });
        Name
    }
}

impl Finalize for Name {}
// SAFETY: no GC pointers inside.
unsafe impl Trace for Name {
    unsafe_empty_trace!();
}

class! {
    class Animal {
        static count: Name;
        public static fn reset() { Animal::count = Name; }
    }
    class Dog extends Animal {
        public static fn count() -> Name { Dog::count }
    }
}

#[roost::main]
fn main() {
    ON_CLONE.with(|f| *f.borrow_mut() = Some(Box::new(Animal::reset)));
    let _ = Dog::count();
    println!("unreachable");
}
