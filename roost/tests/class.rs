//! `class!`: the Java-style class DSL, end to end.
//!
//! Hierarchy used throughout (three levels, plus a sibling):
//!
//! ```text
//! Animal ── Dog ── Puppy
//!        └─ Cat
//! ```
//!
//! Like any user code, nothing below mentions `dyn`, `Gc`, `GcCell`, `borrow` or `&self`.

use std::cell::RefCell;
use std::hint::black_box;

use roost::gc::{Counter, Probe};
use roost::prelude::*;

thread_local! {
    /// What the constructors and `init` overrides did, in order.
    static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn log(s: String) {
    LOG.with(|l| l.borrow_mut().push(s));
}

fn take_log() -> Vec<String> {
    LOG.with(|l| std::mem::take(&mut *l.borrow_mut()))
}

/// `ROOST_SKIP_COLLECT=1` turns collections in this file into no-ops (CI checks that the
/// cycle test then fails, i.e. that it really depends on the collector).
fn collect() {
    if std::env::var_os("ROOST_SKIP_COLLECT").is_none() {
        roost::gc::collect();
    }
}

class! {
    /// The root of the test hierarchy.
    public class Animal {
        protected name: String;
        private legs: i32 = 4;
        protected friend: Option<Ref<Animal>>;
        protected renames: i32;
        protected counter: i64 = 10;
        probe: Option<Probe>;

        public Animal(name: String) {
            log(format!("Animal({name}) sees tricks={}", this.describe_tricks()));
            this.name = name;
            this.init();
        }

        public fn name() -> String { this.name }
        public fn legs() -> i32 { this.legs }
        public fn speak() -> String { format!("{} makes a sound", this.name) }
        public fn describe() -> String { format!("[{}] {}", this.kind(), this.speak()) }
        public fn kind() -> String { "animal".to_string() }
        protected fn init() { log("Animal.init".into()); }
        protected fn describe_tricks() -> String { "n/a".into() }

        /// The spike's panic: base-class code that writes a field and then makes a virtual
        /// call whose override reads and writes that same field.
        public fn rename(name: String) {
            this.name = name;
            this.on_renamed();
            this.renames += 1;
        }
        protected fn on_renamed() {}

        /// `+=` whose right-hand side is a virtual call that itself updates the field.
        public fn bump_counter() -> i64 {
            this.counter += this.step();
            this.counter
        }
        protected fn step() -> i64 { 1 }

        public fn counter() -> i64 { this.counter }
        public fn renames() -> i32 { this.renames }

        public fn befriend(other: Ref<Animal>) { this.friend = Some(other); }
        public fn friend() -> Option<Ref<Animal>> { this.friend }
        public fn attach(probe: Probe) { this.probe = Some(probe); }

        public fn fields_roundtrip() -> String {
            this.legs = this.legs * 2;
            this.legs -= 1;
            this.legs *= 3;
            this.legs <<= 1;
            this.legs %= 100;
            this.name += "!";
            let len = this.name.len();
            format!("{} {} {}", this.legs, this.name, len)
        }

        public fn shout() -> String { this.loud(this.name) }
        private fn loud(s: String) -> String { s.to_uppercase() }

        public static fn kingdom() -> String { "Animalia".into() }
        public fn kingdom_of() -> String { Self::kingdom() }
    }

    public class Dog extends Animal {
        protected tricks: i32;
        private sound: String = "woof".to_string();

        public Dog(name: String) {
            super(name);
            log(format!("Dog body sees tricks={}", this.tricks));
        }

        @Override public fn speak() -> String { super.speak() + ", " + &this.sound }
        @Override public fn kind() -> String { "dog".into() }
        @Override protected fn init() {
            log("Dog.init".into());
            this.tricks = 1;
        }
        @Override protected fn describe_tricks() -> String { this.tricks.to_string() }
        @Override protected fn on_renamed() {
            // Reads and writes the field the base method just wrote.
            this.name = format!("{} the dog", this.name);
        }
        @Override protected fn step() -> i64 {
            this.counter += 100;
            5
        }

        public fn tricks() -> i32 { this.tricks }
        public fn learn() { this.tricks += 1; }
        public fn fetch() -> String { format!("{} fetches", this.name) }
    }

    public class Puppy extends Dog {
        private owner: Option<Ref<Dog>>;
        private age: u8 = 1;

        public Puppy(name: String, owner: Option<Ref<Dog>>) {
            super(name);
            this.owner = owner;
            log("Puppy body".into());
        }

        @Override public fn speak() -> String { super.speak() + " (squeaky)" }
        @Override public fn fetch() -> String { super.fetch() + " clumsily" }
        @Override protected fn init() {
            super.init();
            log("Puppy.init".into());
            this.tricks += 10;
        }

        public fn owner() -> Option<Ref<Dog>> { this.owner }
        public fn age() -> u8 { this.age }
    }

    // No constructor: an implicit no-arg one that calls `super()`... which `Animal` doesn't
    // have, so Cat declares one.
    class Cat extends Animal {
        Cat() { super("Cat".to_string()); }
        @Override public fn speak() -> String { "meow".into() }
    }

    /// A class without a declared constructor or superclass.
    public class Counter2 {
        value: i32;
        public fn inc() -> i32 { this.value += 1; this.value }
    }
}

/// Compiled once; which `speak` runs is decided by the object's vtable at run time.
#[inline(never)]
fn speak_via_vtable(a: &Ref<Animal>) -> String {
    a.speak()
}

fn zoo() -> Vec<Ref<Animal>> {
    vec![
        Animal::new("Blob".into()),
        Dog::new("Rex".into()),
        Puppy::new("Bit".into(), None),
        Cat::new(),
    ]
}

#[test]
fn virtual_dispatch_over_three_levels() {
    let speeches: Vec<String> = black_box(zoo()).iter().map(speak_via_vtable).collect();
    assert_eq!(
        speeches,
        [
            "Blob makes a sound",
            "Rex makes a sound, woof",
            "Bit makes a sound, woof (squeaky)",
            "meow",
        ]
    );
    // A base-class method calling virtual methods on `this` reaches the overrides.
    let kinds: Vec<String> = zoo().iter().map(|a| a.describe()).collect();
    assert_eq!(kinds[0], "[animal] Blob makes a sound");
    assert_eq!(kinds[2], "[dog] Bit makes a sound, woof (squeaky)");

    // Through a `Ref<Dog>` too, for a method Dog introduces and Puppy overrides.
    let dogs: Vec<Ref<Dog>> = vec![Dog::new("Rex".into()), Puppy::new("Bit".into(), None)];
    let fetches: Vec<String> = dogs.iter().map(|d| d.fetch()).collect();
    assert_eq!(fetches, ["Rex fetches", "Bit fetches clumsily"]);
}

#[test]
fn super_calls_reach_the_parent_implementation() {
    // Puppy.speak -> super (Dog.speak) -> super (Animal.speak): each level adds its part.
    let p = Puppy::new("Bit".into(), None);
    assert_eq!(p.speak(), "Bit makes a sound, woof (squeaky)");
    // super.init() in Puppy runs Dog's init, not Animal's.
    take_log();
    let _ = Puppy::new("Bit".into(), None);
    assert!(take_log().iter().any(|s| s == "Dog.init"));
}

#[test]
fn virtual_call_in_a_constructor_lands_in_the_subclass() {
    take_log();
    let d = Dog::new("Rex".into());
    assert_eq!(
        take_log(),
        [
            // Animal's constructor body runs first, but `this` is already the Dog, so the
            // virtual call reaches Dog's override (whose field has its default value: 0).
            "Animal(Rex) sees tricks=0",
            "Dog.init",
            "Dog body sees tricks=1",
        ]
    );
    assert_eq!(
        d.tricks(),
        1,
        "set by Dog.init, called from Animal's constructor"
    );

    let p = Puppy::new("Bit".into(), None);
    assert_eq!(
        take_log(),
        [
            "Animal(Bit) sees tricks=0",
            "Dog.init",
            "Puppy.init",
            "Dog body sees tricks=11",
            "Puppy body",
        ]
    );
    assert_eq!(p.tricks(), 11);

    let a = Animal::new("Blob".into());
    assert_eq!(take_log(), ["Animal(Blob) sees tricks=n/a", "Animal.init"]);
    assert_eq!(a.name(), "Blob");
}

#[test]
fn field_defaults_and_initializers() {
    let p = Puppy::new("Bit".into(), None);
    assert_eq!(p.legs(), 4, "`= 4` in Animal");
    assert_eq!(p.age(), 1, "`= 1` in Puppy");
    assert_eq!(p.renames(), 0, "Default::default()");
    assert!(p.friend().is_none());
    assert_eq!(Dog::new("Rex".into()).speak(), "Rex makes a sound, woof");
    let c = Counter2::new();
    assert_eq!((c.inc(), c.inc()), (1, 2));
}

#[test]
fn field_read_write_and_compound_assignment() {
    let a = Animal::new("Blob".into());
    // legs: 4 -> *2 = 8 -> -1 = 7 -> *3 = 21 -> <<1 = 42 -> %100 = 42; name += "!"
    assert_eq!(a.fields_roundtrip(), "42 Blob! 5");
    assert_eq!(a.legs(), 42);
    assert_eq!(a.name(), "Blob!");

    let d = Dog::new("Rex".into());
    d.learn();
    d.learn();
    assert_eq!(d.tricks(), 3);
}

#[test]
fn base_method_calling_an_override_that_touches_the_same_field_does_not_panic() {
    // Animal.rename writes `name`, then calls on_renamed; Dog's override reads and writes
    // `name`. In the spike this was `GcCell<T> already mutably borrowed`.
    let d: Ref<Animal> = Dog::new("Rex".into());
    d.rename("Max".into());
    assert_eq!(d.name(), "Max the dog");
    assert_eq!(d.renames(), 1);

    let p: Ref<Animal> = Puppy::new("Bit".into(), None);
    p.rename("Tiny".into());
    assert_eq!(p.name(), "Tiny the dog");
}

#[test]
fn compound_assignment_evaluates_like_java() {
    // `this.counter += this.step()`: Java reads the field, then evaluates the right side,
    // then writes. Dog.step adds 100 to the counter in between; that write is overwritten.
    let a = Animal::new("Blob".into());
    assert_eq!(a.bump_counter(), 11);
    let d = Dog::new("Rex".into());
    assert_eq!(d.bump_counter(), 15);
    assert_eq!(d.counter(), 15);
}

#[test]
fn private_and_static_methods() {
    assert_eq!(Dog::new("Rex".into()).shout(), "REX");
    assert_eq!(Animal::kingdom(), "Animalia");
    assert_eq!(Puppy::new("Bit".into(), None).kingdom_of(), "Animalia");
}

#[test]
fn instanceof_and_cast() {
    let animals = zoo();
    let [animal, dog, puppy, cat] = &animals[..] else {
        unreachable!()
    };

    assert!(puppy.instanceof::<Puppy>());
    assert!(puppy.instanceof::<Dog>());
    assert!(puppy.instanceof::<Animal>());
    assert!(dog.instanceof::<Dog>());
    assert!(!dog.instanceof::<Puppy>());
    assert!(!animal.instanceof::<Dog>());
    assert!(!cat.instanceof::<Dog>());

    // Down to a non-leaf class: the result dispatches to the runtime class.
    let as_dog: Ref<Dog> = puppy.cast::<Dog>().expect("a Puppy is a Dog");
    assert_eq!(as_dog.fetch(), "Bit fetches clumsily");
    assert!(Ref::ptr_eq(&as_dog, puppy));
    let as_puppy: Ref<Puppy> = as_dog.cast::<Puppy>().expect("and a Puppy");
    assert_eq!(as_puppy.age(), 1);
    assert_eq!(dog.cast::<Dog>().unwrap().fetch(), "Rex fetches");

    // Failures are None.
    assert!(animal.cast::<Dog>().is_none());
    assert!(cat.cast::<Dog>().is_none());
    assert!(dog.cast::<Puppy>().is_none());
    assert!(dog.cast::<Cat>().is_none());

    // Upcasts are implicit; also through cast, and to Object.
    let up: Ref<Animal> = as_puppy.clone();
    assert!(Ref::ptr_eq(&up, puppy));
    assert!(as_puppy.cast::<Animal>().is_some());
    let o: Ref<dyn Object> = as_puppy;
    assert_eq!(o.class_name(), "Puppy");
    assert!(o.cast::<Dog>().is_some());
}

#[test]
fn ptr_eq_is_reference_identity() {
    let a = Dog::new("Rex".into());
    let b = Dog::new("Rex".into());
    let a_as_animal: Ref<Animal> = a.clone();
    assert!(Ref::ptr_eq(&a, &a_as_animal));
    assert!(!Ref::ptr_eq(&a, &b), "equal contents, different objects");
    let back = a_as_animal.cast::<Dog>().unwrap();
    assert!(Ref::ptr_eq(&back, &a));
}

#[test]
fn class_name_and_to_string() {
    let p: Ref<Animal> = Puppy::new("Bit".into(), None);
    assert_eq!(p.class_name(), "Puppy");
    assert_eq!(p.to_string(), format!("Puppy@{:x}", p.hash_code()));
}

#[test]
fn ref_fields_hold_references_not_copies() {
    let rex = Dog::new("Rex".into());
    let p = Puppy::new("Bit".into(), Some(rex.clone()));
    let owner = p.owner().unwrap();
    assert!(Ref::ptr_eq(&owner, &rex));
    // Reading a Ref field copies the reference; the object is shared.
    owner.learn();
    assert_eq!(rex.tricks(), 2);
}

#[test]
fn cycle_across_three_levels_is_collected() {
    let counter = Counter::new();
    {
        let a = Animal::new("Blob".into());
        let d = Dog::new("Rex".into());
        let p = Puppy::new("Bit".into(), Some(d.clone())); // p -> d via a Ref<Dog> field
        let all: [Ref<Animal>; 3] = [a.clone(), d.clone().upcast(), p.clone().upcast()];
        for x in &all {
            x.attach(counter.probe());
        }
        drop(all);
        // a -> d -> p -> a, plus p -> d.
        a.befriend(d.clone());
        d.befriend(p.clone());
        p.befriend(a.clone());

        // Still reachable from the stack: nothing is freed.
        collect();
        assert_eq!(counter.dropped(), 0);
        assert_eq!(a.friend().unwrap().friend().unwrap().name(), "Bit");
    }
    assert_eq!(
        (counter.finalized(), counter.dropped()),
        (0, 0),
        "reference counting alone never frees a cycle"
    );
    collect();
    assert_eq!((counter.finalized(), counter.dropped()), (3, 3));
}
