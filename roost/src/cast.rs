//! The machinery behind `Ref::cast` / `Ref::instance_of`.
//!
//! The problem: given a `Ref<dyn Animal>` pointing at a `Puppy`, produce a `Ref<dyn Dog>`.
//! That needs `Puppy`'s vtable *for `dyn Dog`*, and only code that knows the concrete type
//! `Puppy` can name it. So the cast is a virtual call ([`Object::__roost_casts`]) into the
//! concrete class, which "offers" itself as each type it can be viewed as; the request
//! keeps the offer whose type is the one asked for.
//!
//! The requested type travels as an `Option<*const U>` behind `&mut dyn Any`: an offer of
//! type `V` fills it only if `Option<*const V>` *is* that type, i.e. `V == U`.
//!
//! Soundness doesn't rely on `__roost_casts` being written correctly (it is a safe method).
//! Every offer records the concrete type and address it was made from, and `Ref::cast`
//! only uses the pointer if those are exactly the runtime class and address of the object
//! being cast. An offer made from a field or from another object is therefore ignored; and
//! an offer of the object itself, coerced with `Unsize`, is exactly the pointer a direct
//! `Gc<Puppy>` -> `Gc<dyn Dog>` coercion would have produced.

use std::any::{Any, TypeId};
use std::marker::Unsize;

use crate::object::Object;

/// A pending `cast::<U>()` / `instance_of::<U>()`, handed to [`Object::__roost_casts`].
pub struct CastRequest<'a> {
    /// `&mut Option<*const U>` for the requested `U`.
    slot: &'a mut dyn Any,
    /// Concrete type and address of whatever filled `slot`.
    source: Option<(TypeId, *const ())>,
}

impl<'a> CastRequest<'a> {
    pub(crate) fn new<U: ?Sized + 'static>(slot: &'a mut Option<*const U>) -> Self {
        CastRequest { slot, source: None }
    }

    /// Offers `this`, viewed as the class trait object `U` (e.g. `dyn Dog`).
    ///
    /// `C: Unsize<U>` means `U` is a trait object type that `C` implements, so `this` and
    /// the returned pointer are the same object. (The `Unsize` bound needs no feature gate
    /// at the call site.)
    pub fn offer<U, C>(&mut self, this: &C) -> &mut Self
    where
        U: ?Sized + 'static,
        C: Object + Unsize<U>,
    {
        let as_u: &U = this;
        // Explicit turbofish: left to inference, `this` (an argument, hence a coercion site)
        // gets unsized to `&U` as well, via the `C: Unsize<U>` bound in scope.
        self.fill::<U, C>(as_u, this)
    }

    /// Offers `this` as its own concrete class (`Ref<Puppy>`).
    pub fn offer_class<C: Object>(&mut self, this: &C) -> &mut Self {
        self.fill::<C, C>(this, this)
    }

    fn fill<U: ?Sized + 'static, C: Object>(&mut self, as_u: &U, this: &C) -> &mut Self {
        if let Some(slot @ None) = self.slot.downcast_mut::<Option<*const U>>() {
            *slot = Some(as_u);
            self.source = Some((TypeId::of::<C>(), (this as *const C).cast()));
        }
        self
    }

    /// Who filled the slot, if anyone: `(concrete type, address)`.
    pub(crate) fn source(&self) -> Option<(TypeId, *const ())> {
        self.source
    }
}

#[cfg(test)]
mod tests {
    use crate::gc::{self, Counter, Finalize, GcCell, Trace};
    use crate::test_classes::*;
    use crate::{CastRequest, Object, Ref};

    #[test]
    fn cast_to_a_non_leaf_class_trait() {
        // Static type Animal, runtime class Puppy (a subclass of Dog).
        let a: Ref<dyn Animal> = puppy("Rex");
        let d: Ref<dyn Dog> = a.cast::<dyn Dog>().expect("a Puppy is a Dog");
        // Same object, and calls through the new reference dispatch to Puppy.
        assert!(Ref::ptr_eq(&a, &d));
        assert_eq!(d.fetch(), "Rex fetches clumsily");
        assert_eq!(d.speak(), "Rex says yip");

        // An instance of the non-leaf class itself.
        let m: Ref<dyn Animal> = mutt("Fido");
        let d = m.cast::<dyn Dog>().expect("a Mutt is a Dog");
        assert_eq!(d.fetch(), "Fido fetches"); // Dog's default
    }

    #[test]
    fn failed_cast_returns_none() {
        let c: Ref<dyn Animal> = cat("Tom");
        let p: Ref<dyn Animal> = plain("Blob");
        assert!(c.cast::<dyn Dog>().is_none(), "a Cat is not a Dog");
        assert!(p.cast::<dyn Dog>().is_none(), "a plain Animal is not a Dog");
        assert!(c.cast::<Puppy>().is_none());

        let d: Ref<dyn Animal> = puppy("Rex");
        assert!(d.cast::<Mutt>().is_none(), "a Puppy is a Dog, not a Mutt");
        assert!(d.cast::<Cat>().is_none());
        // Types outside the hierarchy.
        assert!(d.cast::<String>().is_none());
        assert!(d.cast::<dyn std::fmt::Debug>().is_none());
    }

    #[test]
    fn cast_to_the_concrete_class() {
        let a: Ref<dyn Animal> = puppy("Rex");
        let p: Ref<Puppy> = a.cast::<Puppy>().unwrap();
        assert_eq!(p.name, "Rex");
        assert!(Ref::ptr_eq(&p, &a));
    }

    #[test]
    fn cast_from_the_root_and_upcasts() {
        let o: Ref<dyn Object> = puppy("Rex");
        assert_eq!(o.cast::<dyn Dog>().unwrap().fetch(), "Rex fetches clumsily");
        assert_eq!(o.cast::<dyn Animal>().unwrap().speak(), "Rex says yip");
        let d: Ref<dyn Dog> = o.cast::<dyn Dog>().unwrap();
        assert!(d.cast::<dyn Object>().is_some());
        assert!(d.cast::<Puppy>().is_some());
    }

    #[test]
    fn instance_of() {
        let p: Ref<dyn Animal> = puppy("Rex");
        assert!(p.instance_of::<Puppy>());
        assert!(p.instance_of::<dyn Dog>());
        assert!(p.instance_of::<dyn Animal>());
        assert!(p.instance_of::<dyn Object>());
        assert!(!p.instance_of::<Mutt>());
        assert!(!p.instance_of::<Cat>());

        let c: Ref<dyn Animal> = cat("Tom");
        assert!(!c.instance_of::<dyn Dog>());
        assert!(c.instance_of::<dyn Animal>());
    }

    #[test]
    fn cast_result_is_a_root_of_its_own() {
        let counter = Counter::new();
        let d: Ref<dyn Dog> = {
            let a: Ref<dyn Animal> = Ref::new(Puppy {
                name: "Rex".into(),
                probe: Some(counter.probe()),
            });
            a.cast::<dyn Dog>().unwrap()
        }; // the original reference is gone
        gc::collect();
        assert_eq!(counter.dropped(), 0, "the cast result alone keeps it alive");
        assert_eq!(d.fetch(), "Rex fetches clumsily");
        drop(d);
        gc::collect();
        assert_eq!((counter.finalized(), counter.dropped()), (1, 1));
    }

    #[test]
    fn many_casts_balance_their_roots() {
        let counter = Counter::new();
        let a: Ref<dyn Animal> = Ref::new(Puppy {
            name: "Rex".into(),
            probe: Some(counter.probe()),
        });
        for _ in 0..100 {
            let _ = a.cast::<dyn Dog>().unwrap();
            let _ = a.cast::<Puppy>().unwrap();
            let _ = a.cast::<Mutt>();
        }
        drop(a);
        gc::collect();
        assert_eq!(counter.dropped(), 1, "no root leaked by cast");
    }

    // A class whose `__roost_casts` lies: it offers a different object (or a part of
    // itself) instead of itself. `cast` must not trust such an offer.
    #[derive(Trace, Finalize)]
    struct Liar {
        inner: Cat,
        twin: GcCell<Option<Ref<Liar>>>,
    }

    impl Object for Liar {
        fn __roost_casts(&self, req: &mut CastRequest<'_>) {
            match &*self.twin.borrow() {
                // Right class, wrong object.
                Some(twin) => req
                    .offer_class::<Liar>(twin)
                    .offer::<dyn Object, _>(&**twin),
                // An embedded object at a different address and of a different class.
                None => req
                    .offer::<dyn Animal, _>(&self.inner)
                    .offer::<dyn Object, _>(self),
            };
        }
    }

    #[test]
    fn dishonest_offers_are_ignored() {
        let liar: Ref<dyn Object> = Ref::new(Liar {
            inner: Cat { name: "Tom".into() },
            twin: GcCell::new(None),
        });
        assert!(liar.cast::<dyn Animal>().is_none());
        assert!(!liar.instance_of::<dyn Animal>());
        // The honest offer still works.
        assert!(liar.cast::<dyn Object>().is_some());

        let other = Ref::new(Liar {
            inner: Cat { name: "Kit".into() },
            twin: GcCell::new(None),
        });
        let liar = liar.cast::<Liar>();
        assert!(liar.is_none(), "offer_class was never called for itself");
        let liar2 = Ref::new(Liar {
            inner: Cat { name: "Tom".into() },
            twin: GcCell::new(Some(other.clone())),
        });
        let as_obj: Ref<dyn Object> = liar2.clone();
        assert!(
            as_obj.cast::<Liar>().is_none(),
            "offered its twin, not itself"
        );
        assert!(as_obj.cast::<dyn Object>().is_none());
    }
}
