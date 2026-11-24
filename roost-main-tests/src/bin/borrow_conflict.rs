//! A borrow conflict on an inherited field. `this.name` copies the field out while holding a
//! short read borrow; `Name::clone` calls back into the object and writes the same field
//! during that copy. Contrived, but it is the only way left to get a conflict: field access
//! is otherwise short borrows only (see docs/design.md).

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
        protected name: Name;
        public fn name() -> Name { this.name }
        public fn rename() { this.name = Name; }
    }
    class Dog extends Animal {}
}

#[roost::main]
fn main() {
    let dog: Ref<Animal> = new!(Dog());
    let again = dog.clone();
    ON_CLONE.with(|f| *f.borrow_mut() = Some(Box::new(move || again.rename())));
    let _ = dog.name();
    println!("unreachable");
}
