//! The five things the spike has to prove, each with assertions.
#![feature(ptr_metadata)]

use std::hint::black_box;
use std::mem::size_of;

use roost::zoo::{Animal, AnimalBase, Dog, animal_super};
use roost::{Gc, GcExt, probe};

/// `ROOST_SKIP_COLLECT=1` turns every collection in this file into a no-op. Used to show
/// that the cycle tests depend on the collector: with it set, they must fail.
/// (CI runs that and expects failure.)
fn collect() {
    if std::env::var_os("ROOST_SKIP_COLLECT").is_none() {
        roost::force_collect();
    }
}

fn rex() -> Gc<dyn Animal> {
    Dog::create("Rex", "beagle")
}

fn generic() -> Gc<dyn Animal> {
    AnimalBase::create("Blob")
}

// ---------------------------------------------------------------------------------------
// 1. Two-level inheritance: field containment + vtable, referenced as Gc<dyn Animal>.
// ---------------------------------------------------------------------------------------

#[test]
fn one_inheritance_by_field_containment() {
    let dog: Gc<Dog> = Dog::create("Rex", "beagle");
    // Dog's own field and the inherited one, reached directly...
    assert_eq!(dog.breed, "beagle");
    assert_eq!(*dog.base.name.borrow(), "Rex");

    // ...and the inherited one reached through the vtable of a Gc<dyn Animal>.
    let animal: Gc<dyn Animal> = dog.clone();
    assert_eq!(*animal.animal().name.borrow(), "Rex");
    assert!(std::ptr::eq(animal.animal(), &dog.base));

    // Gc<dyn Animal> is a fat pointer: data pointer + vtable pointer.
    assert_eq!(size_of::<Gc<dyn Animal>>(), 2 * size_of::<usize>());
    assert_eq!(size_of::<Gc<Dog>>(), size_of::<usize>());
}

// ---------------------------------------------------------------------------------------
// 2. Overriding + real dynamic dispatch through Gc<dyn Animal>.
// ---------------------------------------------------------------------------------------

/// One non-generic function, compiled once. It cannot know the concrete type, so the only
/// way it can return Dog's text for a dog and Animal's text for an animal is the vtable.
#[inline(never)]
fn speak_via_vtable(animal: &Gc<dyn Animal>) -> String {
    animal.speak()
}

#[test]
fn two_override_dispatches_through_the_vtable() {
    // Which class each element has is decided at runtime (black_box hides the constant).
    let kinds: Vec<bool> = black_box(vec![true, false, true]);
    let zoo: Vec<Gc<dyn Animal>> = kinds
        .iter()
        .map(|&is_dog| if is_dog { rex() } else { generic() })
        .collect();

    let said: Vec<String> = zoo.iter().map(speak_via_vtable).collect();
    assert_eq!(
        said,
        [
            "Rex makes a sound - Woof! (a beagle)",
            "Blob makes a sound",
            "Rex makes a sound - Woof! (a beagle)",
        ]
    );

    // A *base* method calling a virtual one still lands in the subclass.
    assert_eq!(
        zoo[0].describe(),
        "[Dog] Rex makes a sound - Woof! (a beagle)"
    );
    assert_eq!(zoo[1].describe(), "[AnimalBase] Blob makes a sound");

    // The vtable pointer stored in the fat Gc differs per runtime class.
    let vtable = |a: &Gc<dyn Animal>| std::ptr::metadata(Gc::as_ptr(a));
    assert_ne!(vtable(&zoo[0]), vtable(&zoo[1]));
    assert_eq!(vtable(&zoo[0]).size_of(), size_of::<Dog>());
    assert_eq!(vtable(&zoo[1]).size_of(), size_of::<AnimalBase>());
}

// ---------------------------------------------------------------------------------------
// 3. `super` calls.
// ---------------------------------------------------------------------------------------

#[test]
fn three_super_call_reaches_the_base_implementation() {
    let dog = rex();
    // Dog::speak = super.speak() + its own part.
    assert!(dog.speak().starts_with(&animal_super::speak(&*dog)));
    // Calling the base implementation directly on a dog skips the override...
    assert_eq!(animal_super::speak(&*dog), "Rex makes a sound");
    // ...but `this` inside it is still the dog, so virtual calls from there go to Dog.
    assert_eq!(
        animal_super::describe(&*dog),
        "[Dog] Rex makes a sound - Woof! (a beagle)"
    );
}

/// The constructor trap, pinned down. The base constructor calls `speak()` while building
/// the object; since there is no Gc<dyn Animal> yet, that's the *base* speak even for a Dog.
/// (Java would call Dog.speak() there and see `breed == null`; C++ behaves like this.)
#[test]
fn three_b_virtual_call_in_base_constructor_is_not_virtual() {
    let dog = rex();
    assert_eq!(dog.animal().first_words, "Rex makes a sound");
    assert_ne!(dog.animal().first_words, dog.speak());
}

// ---------------------------------------------------------------------------------------
// 4. instanceof / downcast.
// ---------------------------------------------------------------------------------------

#[test]
fn four_instanceof_and_checked_downcast() {
    let dog = rex();
    let blob = generic();

    assert!(dog.is::<Dog>());
    assert!(dog.is::<AnimalBase>(), "a Dog is an Animal");
    assert!(!blob.is::<Dog>());
    assert!(blob.is::<AnimalBase>());

    let as_dog: Gc<Dog> = dog.cast::<Dog>().expect("dog is a Dog");
    assert_eq!(as_dog.breed, "beagle");
    // It's the same object, not a copy.
    let back: Gc<dyn Animal> = as_dog.clone();
    assert!(Gc::ptr_eq(&back, &dog));

    // Failing casts return None instead of panicking.
    assert!(blob.cast::<Dog>().is_none());
    // Exact-class only: there's no separate Gc<AnimalBase> allocation inside a Dog.
    assert!(dog.cast::<AnimalBase>().is_none());
    assert!(blob.cast::<AnimalBase>().is_some());

    assert_eq!(dog.class_name(), "Dog");
    assert_eq!(blob.class_name(), "AnimalBase");
}

#[test]
fn four_b_cast_keeps_the_object_alive_and_rooted() {
    let before = probe::dropped();
    let as_dog = rex().cast::<Dog>().unwrap(); // the Gc<dyn Animal> is gone
    collect();
    assert_eq!(
        probe::dropped(),
        before,
        "the cast result alone must root the object"
    );
    assert_eq!(*as_dog.base.name.borrow(), "Rex");
    drop(as_dog);
    collect();
    assert_eq!(probe::dropped(), before + 1);
}

// ---------------------------------------------------------------------------------------
// 5. Cycles are really collected.
// ---------------------------------------------------------------------------------------

#[test]
fn five_cycle_is_collected() {
    assert_eq!(probe::dropped(), 0);
    {
        let a = rex();
        let b = generic();
        a.clone().befriend(b.clone()); // a -> b -> a
        assert!(Gc::ptr_eq(a.animal().friend.borrow().as_ref().unwrap(), &b));
        assert!(Gc::ptr_eq(b.animal().friend.borrow().as_ref().unwrap(), &a));
    }
    // No outside handle is left. Reference counting alone would leak these two forever:
    assert_eq!(
        probe::dropped(),
        0,
        "nothing frees a cycle before the collector runs"
    );

    collect();

    assert_eq!(probe::finalized(), 2, "both objects finalized");
    assert_eq!(probe::dropped(), 2, "both objects freed");
}

#[test]
fn five_b_self_cycle_is_collected_and_reachable_cycles_are_not() {
    // Reachable cycle: c <-> d, still rooted through `c`.
    let c = rex();
    let d = generic();
    c.clone().befriend(d.clone());
    drop(d);

    // Unreachable one-object cycle: e -> e.
    let e = generic();
    e.clone().befriend(e.clone());
    drop(e);

    collect();

    assert_eq!(probe::dropped(), 1, "only the self-cycle is garbage");
    // d is still alive and usable through c.
    assert_eq!(c.greet_friend(), "Rex greets Blob: \"Blob makes a sound\"");

    drop(c);
    collect();
    assert_eq!(probe::dropped(), 3);
}

// ---------------------------------------------------------------------------------------
// Equality is identity.
// ---------------------------------------------------------------------------------------

#[test]
fn identity_is_ptr_eq() {
    let a = rex();
    let b = rex(); // same field values, different object
    let a2 = a.clone();
    assert!(Gc::ptr_eq(&a, &a2));
    assert!(!Gc::ptr_eq(&a, &b));
}

// ---------------------------------------------------------------------------------------
// Evidence for the README: GcCell borrow conflicts.
// ---------------------------------------------------------------------------------------

#[test]
fn borrow_safe_rename_works_for_both_classes() {
    let dog = rex();
    assert_eq!(dog.rename("Max".into()), "Rex now answers to Max");
    let blob = generic();
    assert_eq!(blob.rename("Goo".into()), "");
}

/// The same base method is fine for AnimalBase...
#[test]
fn borrow_conflict_does_not_show_up_in_the_base_class() {
    assert_eq!(generic().rename_holding_borrow("Goo".into()), "");
}

/// ...and panics as soon as a subclass override touches the field the base still borrows.
#[test]
#[should_panic(expected = "GcCell<T> already mutably borrowed")]
fn borrow_conflict_appears_only_once_a_subclass_overrides() {
    rex().rename_holding_borrow("Max".into());
}

/// Holding a shared borrow across a call that mutates the same object.
#[test]
#[should_panic(expected = "GcCell<T> already borrowed")]
fn borrow_conflict_reading_friend_while_befriending() {
    let a = rex();
    let b = generic();
    let current = a.animal().friend.borrow();
    if current.is_none() {
        a.clone().befriend(b);
    }
}
