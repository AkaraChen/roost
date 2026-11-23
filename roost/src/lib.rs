//! roost: Java-style classes on a tracing GC. [`class!`] declares them; the rest of this
//! crate is the runtime that the generated code builds on.
//!
//! * [`Ref<T>`]: a GC reference (`Gc<T>` underneath). `Ref<dyn Animal>` is what a Java
//!   variable of type `Animal` becomes. Clone + Deref, identity via [`Ref::ptr_eq`],
//!   implicit upcasts `Ref<Dog>` -> `Ref<dyn Animal>`.
//! * [`Object`]: the root class trait. `to_string` / `hash_code` / `equals` defaults, `Any`.
//! * [`Ref::instance_of`] / [`Ref::cast`]: `instanceof` and checked casts, to a concrete
//!   class *or* to a non-leaf class trait (`Ref<dyn Animal>` -> `Ref<dyn Dog>`). A class
//!   opts in by listing what it can be viewed as in [`Object::__roost_casts`].
//! * [`gc`]: `collect()`, re-exports of the collector, and finalize/drop counters for tests.
//!
//! Everything is single-threaded: the collector is thread-local and `Ref` is `!Send`.

// `Ref<T>` -> `Ref<U>` unsizing (`CoerceUnsized`), `self: Ref<Self>` receivers on trait
// objects (`DispatchFromDyn`), and the `Unsize` bound behind `CastRequest::offer`.
#![feature(coerce_unsized, dispatch_from_dyn, unsize)]
// `<*const T>::with_metadata_of`, to rebuild a fat pointer in `Ref::cast`.
#![feature(set_ptr_value)]
// Unit tests exercise `self: Ref<Self>` methods.
#![cfg_attr(test, feature(arbitrary_self_types))]

// Generated code names `::roost::...`; this makes that path work inside this crate too.
extern crate self as roost;

mod cast;
pub mod gc;
mod object;
mod reference;
pub mod runtime;
#[cfg(test)]
mod test_classes;

pub use cast::CastRequest;
pub use object::{AsAny, Object};
pub use reference::Ref;
/// Declares Java-style classes.
///
/// ```
/// use roost::prelude::*;
///
/// class! {
///     public class Animal {
///         protected name: String;
///         private legs: i32 = 4;
///
///         public Animal(name: String) {
///             this.name = name;
///             this.init(); // virtual, even in a constructor
///         }
///         public fn speak() -> String { format!("{} makes a sound", this.name) }
///         public fn legs() -> i32 { this.legs }
///         protected fn init() {}
///     }
///
///     public class Dog extends Animal {
///         private tricks: i32;
///         public Dog(name: String) { super(name); }
///         @Override public fn speak() -> String { super.speak() + ", woof" }
///         @Override protected fn init() { this.tricks += 1; }
///         public fn tricks() -> i32 { this.tricks }
///     }
/// }
///
/// let a: Ref<Animal> = Dog::new("Rex");
/// assert_eq!(a.speak(), "Rex makes a sound, woof");
/// assert_eq!(a.legs(), 4);
/// assert!(a.instanceof::<Dog>());
/// let d: Ref<Dog> = a.cast::<Dog>().unwrap();
/// assert_eq!(d.tricks(), 1);
/// assert!(Ref::ptr_eq(&a, &d));
/// ```
///
/// What each piece becomes, and how it differs from Java, is in the README. In short:
/// `Ref<Animal>` is a polymorphic reference (`type Animal = dyn AnimalClass`), methods are
/// virtual unless `private` or `static`, `this.field` is rewritten into short borrows that
/// copy values out, and constructors follow Java's order.
///
/// Reading a field yields a copy, so a `&mut self` method on it does not compile instead
/// of silently changing the copy:
///
/// ```compile_fail,E0596
/// use roost::prelude::*;
/// class! {
///     class Bag {
///         items: Vec<i32>;
///         public fn add(x: i32) { this.items.push(x); }
///     }
/// }
/// ```
///
/// Overriding without `@Override` is an error (and so is `@Override` on a method that
/// overrides nothing):
///
/// ```compile_fail
/// use roost::prelude::*;
/// class! {
///     class A { public fn f() {} }
///     class B extends A { public fn f() {} }
/// }
/// ```
///
/// `private` fields are not visible to subclasses:
///
/// ```compile_fail
/// use roost::prelude::*;
/// class! {
///     class A { private x: i32; }
///     class B extends A { public fn f() -> i32 { this.x } }
/// }
/// ```
pub use roost_macros::class;

/// The program entry point: runs `main` on a thread with a big stack, configures the
/// collector, collects once before exiting, and reports uncaught roost exceptions and an
/// `Err` from `main` like Java does.
///
/// ```no_run
/// #[roost::main(stack = "256MB", gc_threshold = "1MB", gc_stats)]
/// fn main() -> Result<(), std::num::ParseIntError> {
///     let n: i32 = "12x".parse()?; // Exception in thread "main" ParseIntError: invalid digit ...
///     println!("{n}");
///     Ok(())
/// }
/// ```
///
/// Arguments, all optional:
///
/// * `stack = "64MB"`: stack size of the `main` thread (default 64 MB). Units `B`, `KB`,
///   `MB`, `GB` (powers of 1024), or a plain integer of bytes.
/// * `gc_threshold = "1MB"`: GC heap bytes before the first automatic collection.
/// * `gc_stats`: print allocation / collection statistics to stderr on exit.
///
/// `main` must be `fn main()` or `fn main() -> Result<(), E>` with `E: Debug` (shown with
/// `Display` and its `source()` chain when `E` has them). Exit codes: 0, 1 for an `Err` or an
/// uncaught roost exception, 101 for any other panic. See [`runtime`] for the details.
pub use roost_macros::main;

/// `new!(Dog("Rex"))` is `Dog::new("Rex")`: Java's `new Dog("Rex")` as far as a macro can
/// spell it.
///
/// ```
/// use roost::prelude::*;
/// class! {
///     class Animal { name: String; Animal(name: String) { this.name = name; }
///                    public fn name() -> String { this.name } }
///     class Dog extends Animal { Dog(name: String) { super(name); } }
/// }
/// let d = new!(Dog("Rex"));
/// let a = new!(Dog("Rex")) as Ref<Animal>; // upcast: `as` performs the coercion
/// assert_eq!(a.name(), d.name());
/// ```
#[macro_export]
macro_rules! new {
    ($($class:ident)::+ ( $($arg:expr),* $(,)? )) => {
        <$($class)::+>::new($($arg),*)
    };
    ($($t:tt)*) => {
        ::core::compile_error!("expected `new!(ClassName(arguments..))`, e.g. `new!(Dog(\"Rex\"))`")
    };
}

/// `use roost::prelude::*;` brings in what code using classes needs.
pub mod prelude {
    pub use crate::{Object, Ref, class, new};
}

/// Support for code generated by `class!`. Not a public API.
#[doc(hidden)]
pub mod __private {
    use crate::gc::{GcCell, Trace};

    use crate::runtime::{Exception, ExceptionKind, throw};

    /// A field's storage and its name, for error messages.
    pub struct Field<'a, T: 'static> {
        cell: &'a GcCell<T>,
        class: &'static str,
        name: &'static str,
    }

    /// `this.f` of class `class`: what `get` / `set` take.
    #[inline]
    pub fn field<'a, T>(
        cell: &'a GcCell<T>,
        class: &'static str,
        name: &'static str,
    ) -> Field<'a, T> {
        Field { cell, class, name }
    }

    #[cold]
    #[inline(never)]
    #[track_caller]
    fn conflict(class: &str, name: &str, what: &str) -> ! {
        throw(Exception {
            kind: ExceptionKind::BorrowConflict,
            message: format!("field {class}.{name} {what}"),
        })
    }

    /// A field read: a copy of the value. The borrow ends before this returns, so no
    /// `GcCell` guard can be alive while user code (a virtual call) runs.
    #[inline]
    #[track_caller]
    pub fn get<T: Trace + Clone>(f: Field<'_, T>) -> T {
        match f.cell.try_borrow() {
            Ok(v) => v.clone(),
            Err(_) => conflict(f.class, f.name, "cannot be read while it is being written"),
        }
    }

    /// Fails to compile unless `T` can be a field of a GC-managed object.
    pub fn assert_trace<T: Trace>() {}

    /// A field write. The old value is dropped after the borrow has ended.
    #[inline]
    #[track_caller]
    pub fn set<T: Trace>(f: Field<'_, T>, value: T) {
        let old = match f.cell.try_borrow_mut() {
            Ok(mut v) => std::mem::replace(&mut *v, value),
            Err(_) => conflict(
                f.class,
                f.name,
                "cannot be written while it is being read (a read copies the value out; \
                 user code ran during that copy and wrote the same field)",
            ),
        };
        drop(old);
    }
}
