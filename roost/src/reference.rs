//! `Ref<T>`: a reference to a GC-managed object.

use std::any::Any;
use std::fmt;
use std::marker::Unsize;
use std::ops::{CoerceUnsized, Deref, DispatchFromDyn};

use crate::cast::CastRequest;
use crate::gc::{Finalize, Gc, Trace};
use crate::object::Object;

/// A reference to a garbage-collected object: what a Java variable of class type holds.
///
/// * `Ref<Dog>`: a reference whose static type is a concrete class.
/// * `Ref<dyn Animal>`: a reference to any object whose class extends `Animal`. Method
///   calls go through the vtable. `Ref<Dog>` converts to it implicitly (unsizing).
///
/// Cloning copies the reference, not the object. There is no `PartialEq`: comparing
/// references is [`Ref::ptr_eq`], comparing objects is [`Object::equals`].
pub struct Ref<T: ?Sized + 'static>(Gc<T>);

impl<T: Trace> Ref<T> {
    /// Allocates `value` on the GC heap. May run a collection first.
    pub fn new(value: T) -> Ref<T> {
        Ref(Gc::new(value))
    }
}

impl<T: ?Sized> Ref<T> {
    /// Wraps an existing `Gc`.
    pub fn from_gc(gc: Gc<T>) -> Ref<T> {
        Ref(gc)
    }

    /// The underlying `Gc`.
    pub fn as_gc(this: &Ref<T>) -> &Gc<T> {
        &this.0
    }

    /// Unwraps into the underlying `Gc`.
    pub fn into_gc(this: Ref<T>) -> Gc<T> {
        this.0
    }

    /// An explicit upcast: `Dog::new(..).upcast::<Animal>()`. Same as the implicit
    /// coercion (`let a: Ref<Animal> = dog;`), for places where no type is expected.
    /// (A `From` impl for this would overlap with `impl<T> From<T> for T`.)
    pub fn upcast<U: ?Sized>(self) -> Ref<U>
    where
        T: Unsize<U>,
    {
        self
    }

    /// Java's `a == b` on references: do both point to the same object?
    ///
    /// The two sides may have different static types (`Ref<Dog>` vs `Ref<dyn Animal>`):
    /// only the object's address is compared, never the vtable. (Comparing fat pointers
    /// with `==` also compares vtable pointers, which can differ for the same object.)
    pub fn ptr_eq<U: ?Sized>(this: &Ref<T>, other: &Ref<U>) -> bool {
        // Every object has its own allocation with a non-empty header in front, so even
        // zero-sized objects have distinct addresses.
        std::ptr::addr_eq(Gc::as_ptr(&this.0), Gc::as_ptr(&other.0))
    }
}

impl<T: ?Sized + Object> Ref<T> {
    /// Java's `x instanceof U`. `U` is a concrete class (`Puppy`) or a class trait object
    /// (`dyn Dog`); `true` for subclasses too.
    pub fn instance_of<U: ?Sized + 'static>(&self) -> bool {
        self.lookup::<U>().is_some()
    }

    /// Same as [`instance_of`](Ref::instance_of), spelled like Java's operator.
    pub fn instanceof<U: ?Sized + 'static>(&self) -> bool {
        self.instance_of::<U>()
    }

    /// Java's `(U) x`, checked: the same object as a `Ref<U>`, or `None` if it isn't one.
    ///
    /// `U` can be the concrete runtime class (`Ref<dyn Animal>` -> `Ref<Puppy>`) or the
    /// trait of any class the object's class extends, leaf or not (`Ref<dyn Animal>` ->
    /// `Ref<dyn Dog>`). Upcasts work too, but the implicit coercion is simpler for those.
    pub fn cast<U: ?Sized + 'static>(&self) -> Option<Ref<U>> {
        let target = self.lookup::<U>()?;
        // `into_raw` keeps the root that `clone` added, `from_raw` takes it over (like
        // `Rc::into_raw` / `Rc::from_raw`): the result is a new, rooted reference.
        let raw: *const T = Gc::into_raw(self.0.clone());
        // Address and provenance from `into_raw`, only the metadata (vtable) from `target`.
        // `target` itself was derived from a `&U` to the value, so its provenance doesn't
        // cover the header in front of it, which `from_raw` steps back to (Miri flags that).
        let raw: *const U = raw.with_metadata_of(target);
        // SAFETY: `lookup` checked that `target` was made from this very object, at its
        // own address and with its exact runtime class `C`, and unsized from `&C` (see
        // `CastRequest`). So `raw` is the pointer `Gc::into_raw` would return for a
        // `Gc<C>` coerced to `Gc<U>`: `from_raw`, which finds the allocation header from
        // the value's size and alignment (both read from the metadata, so both `C`'s),
        // recovers the very `GcBox` this reference points to.
        Some(Ref(unsafe { Gc::from_raw(raw) }))
    }

    /// Asks the object for a `*const U` view of itself, and checks the answer.
    fn lookup<U: ?Sized + 'static>(&self) -> Option<*const U> {
        let mut slot: Option<*const U> = None;
        let mut request = CastRequest::new(&mut slot);
        let object: &T = self;
        object.__roost_casts(&mut request);
        let (class, addr) = request.source()?;
        // Through the vtable: the concrete class, even when `T` is `dyn Animal`.
        let runtime_class = <dyn Any>::type_id(object.as_any());
        let object_addr = (object as *const T).cast::<()>();
        if class != runtime_class || addr != object_addr {
            return None;
        }
        slot
    }
}

impl<T: ?Sized> Clone for Ref<T> {
    fn clone(&self) -> Self {
        Ref(self.0.clone())
    }
}

impl<T: ?Sized> Deref for Ref<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

// `Ref<Dog>` -> `Ref<dyn Animal>`, `Ref<dyn Dog>` -> `Ref<dyn Animal>` (trait upcasting).
impl<T: ?Sized + Unsize<U>, U: ?Sized> CoerceUnsized<Ref<U>> for Ref<T> {}

// Lets a class trait have `self: Ref<Self>` methods and still be dyn-compatible.
impl<T: ?Sized + Unsize<U>, U: ?Sized> DispatchFromDyn<Ref<U>> for Ref<T> {}

impl<T: ?Sized> Finalize for Ref<T> {}

// SAFETY: forwards everything to the wrapped `Gc`, the only field. (Written out rather than
// with gc's `custom_trace!`, whose expansion trips `unsafe_op_in_unsafe_fn` in edition 2024.)
unsafe impl<T: ?Sized + Trace> Trace for Ref<T> {
    unsafe fn trace(&self) {
        unsafe { self.0.trace() }
    }
    unsafe fn root(&self) {
        unsafe { self.0.root() }
    }
    unsafe fn unroot(&self) {
        unsafe { self.0.unroot() }
    }
    fn finalize_glue(&self) {
        Finalize::finalize(self);
        self.0.finalize_glue();
    }
}

/// `toString()`.
impl<T: ?Sized + Object> fmt::Display for Ref<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&Object::to_string(&**self))
    }
}

impl<T: ?Sized + Object> fmt::Debug for Ref<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Ref({})", Object::to_string(&**self))
    }
}

#[cfg(test)]
mod tests {
    use std::mem::size_of;

    use super::*;
    use crate::gc::{self, Counter, GcCell};
    use crate::test_classes::*;

    #[test]
    fn new_deref_and_clone_share_the_object() {
        let a = plain("Blob");
        assert_eq!(a.name, "Blob");
        let b = a.clone();
        assert!(Ref::ptr_eq(&a, &b));
        *a.friend.borrow_mut() = Some(puppy("Rex"));
        // The write through `a` is visible through `b`: one object, two references.
        assert_eq!(b.friend.borrow().as_ref().unwrap().name(), "Rex");
    }

    #[test]
    fn ptr_eq_is_identity_not_value_equality() {
        let a = plain("Blob");
        let b = plain("Blob");
        assert!(!Ref::ptr_eq(&a, &b));
        assert!(Ref::ptr_eq(&a, &a.clone()));
    }

    #[test]
    fn ptr_eq_across_static_types() {
        let p = puppy("Rex");
        let as_dog: Ref<dyn Dog> = p.clone();
        let as_animal: Ref<dyn Animal> = p.clone();
        let as_object: Ref<dyn Object> = p.clone();
        assert!(Ref::ptr_eq(&p, &as_dog));
        assert!(Ref::ptr_eq(&as_dog, &as_animal));
        assert!(Ref::ptr_eq(&as_animal, &as_object));
        assert!(!Ref::ptr_eq(&as_animal, &mutt("Rex")));
    }

    #[test]
    fn upcasts_are_implicit_and_dispatch_through_the_vtable() {
        let p: Ref<Puppy> = puppy("Rex");
        let d: Ref<dyn Dog> = p; // class -> its class trait
        let a: Ref<dyn Animal> = d.clone(); // trait upcasting: dyn Dog -> dyn Animal
        let o: Ref<dyn Object> = a.clone(); // ... -> dyn Object
        assert_eq!(d.fetch(), "Rex fetches clumsily");
        assert_eq!(a.speak(), "Rex says yip");
        assert!(o.class_name().ends_with("Puppy"));

        assert_eq!(size_of::<Ref<Puppy>>(), size_of::<usize>());
        assert_eq!(size_of::<Ref<dyn Animal>>(), 2 * size_of::<usize>());
    }

    #[test]
    fn self_ref_receiver_dispatches_on_a_trait_object() {
        let a: Ref<dyn Animal> = cat("Tom");
        assert_eq!(a.clone().name_via_ref(), "Tom");
    }

    #[test]
    fn gc_round_trip() {
        let a = plain("Blob");
        let g = Ref::into_gc(a.clone());
        assert!(gc::Gc::ptr_eq(&g, Ref::as_gc(&a)));
        assert!(Ref::ptr_eq(&Ref::from_gc(g), &a));
    }

    #[test]
    fn refs_are_traced_so_cycles_through_them_are_collected() {
        let counter = Counter::new();
        let new = |name: &str| {
            Ref::new(PlainAnimal {
                name: name.into(),
                friend: GcCell::new(None),
                probe: Some(counter.probe()),
            })
        };
        let a = new("a");
        let b = new("b");
        *a.friend.borrow_mut() = Some(b.clone());
        *b.friend.borrow_mut() = Some(a.clone());

        // While `a` is rooted, both survive (b is reached by tracing a's `Ref` field).
        drop(b);
        gc::collect();
        assert_eq!(counter.dropped(), 0);
        assert_eq!(a.friend.borrow().as_ref().unwrap().name(), "b");

        drop(a);
        assert_eq!(
            counter.dropped(),
            0,
            "a cycle is never freed by refcounting"
        );
        gc::collect();
        assert_eq!((counter.finalized(), counter.dropped()), (2, 2));
    }

    #[test]
    fn display_and_debug_use_to_string() {
        let c: Ref<dyn Animal> = cat("Tom");
        assert_eq!(c.to_string(), "Cat(Tom)"); // virtual: Cat overrides it
        assert_eq!(format!("{c:?}"), "Ref(Cat(Tom))");
    }
}
