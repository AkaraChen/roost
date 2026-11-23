//! roost: Java-style classes on a tracing GC. [`class!`] declares them; the rest of this
//! crate is the runtime that the generated code builds on.
//!
//! * [`Ref<T>`]: a GC reference (`Gc<T>` underneath). `Ref<dyn Animal>` is what a Java
//!   variable of type `Animal` becomes. Clone + Deref, identity via [`Ref::ptr_eq`],
//!   implicit upcasts `Ref<Dog>` -> `Ref<dyn Animal>`.
//! * [`ObjectClass`]: the root class trait (`Ref<Object>`, with `type Object = dyn
//!   ObjectClass`). `to_string` / `hash_code` / `equals` defaults, `Any`.
//! * [`Ref::instance_of`] / [`Ref::cast`]: `instanceof` and checked casts, to a concrete
//!   class *or* to a non-leaf class trait (`Ref<dyn Animal>` -> `Ref<dyn Dog>`). A class
//!   opts in by listing what it can be viewed as in [`ObjectClass::__roost_casts`].
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
#[cfg(test)]
mod test_classes;

pub use cast::CastRequest;
pub use object::{AsAny, Object, ObjectClass};
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
/// let a: Ref<Animal> = Dog::new("Rex".into());
/// assert_eq!(a.speak(), "Rex makes a sound, woof");
/// assert_eq!(a.legs(), 4);
/// assert!(a.instanceof::<Dog>());
/// let d: Ref<Dog> = a.cast::<Dog>().unwrap();
/// assert_eq!(d.tricks(), 1);
/// assert!(Ref::ptr_eq(&a, &d));
/// ```
///
/// Interfaces, abstract classes and static members:
///
/// ```
/// use roost::prelude::*;
///
/// class! {
///     public interface Shape {
///         fn area() -> f64;
///         default fn describe() -> String { format!("area {:.1}", this.area()) }
///     }
///     public abstract class Base implements Shape {
///         private static made: i32;
///         protected Base() { Self::made += 1; }
///         public static fn made() -> i32 { Self::made }
///     }
///     public final class Square extends Base {
///         side: f64;
///         public Square(side: f64) { super(); this.side = side; }
///         @Override public fn area() -> f64 { this.side * this.side }
///         @Override public fn to_string() -> String { format!("Square({})", this.side) }
///     }
/// }
///
/// let s: Ref<Shape> = Square::new(2.0);
/// assert_eq!(s.describe(), "area 4.0");
/// assert_eq!(format!("{s}"), "Square(2)");
/// assert!(s.instanceof::<Base>());
/// assert_eq!(Base::made(), 1);
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

/// `use roost::prelude::*;` brings in what code using classes needs.
pub mod prelude {
    pub use crate::{Object, ObjectClass, Ref, class};
}

/// Support for code generated by `class!`. Not a public API.
#[doc(hidden)]
pub mod __private {
    use crate::gc::{GcCell, Trace};

    /// A field read: a copy of the value. The borrow ends before this returns, so no
    /// `GcCell` guard can be alive while user code (a virtual call) runs.
    #[inline]
    pub fn get<T: Trace + Clone>(cell: &GcCell<T>) -> T {
        cell.borrow().clone()
    }

    /// Fails to compile unless `T` can be a field of a GC-managed object.
    pub fn assert_trace<T: Trace>() {}

    /// A field write. The old value is dropped after the borrow has ended.
    #[inline]
    pub fn set<T: Trace>(cell: &GcCell<T>, value: T) {
        let old = std::mem::replace(&mut *cell.borrow_mut(), value);
        drop(old);
    }

    /// A static field: a thread-local `GcCell` (the collector is thread-local too).
    pub type Static<T> = std::thread::LocalKey<GcCell<T>>;

    /// A static field read: a copy, like [`get`].
    #[inline]
    pub fn get_static<T: Trace + Clone>(key: &'static Static<T>) -> T {
        key.with(get)
    }

    /// A static field write, like [`set`].
    #[inline]
    pub fn set_static<T: Trace>(key: &'static Static<T>, value: T) {
        key.with(|cell| set(cell, value));
    }

    pub use crate::object::{default_to_string, identity_equals, identity_hash_code};
}
