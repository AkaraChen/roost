//! `class!` beyond the basics: interfaces, abstract, final, static, and the `Object`
//! methods. (Compile-time errors are in `tests/compile_fail.rs`.)
//!
//! ```text
//! interface Named                   interface Comparable
//!     └── interface Pet (default greet)
//!
//! abstract class Animal implements Named
//!     ├── class Dog extends Animal implements Pet, Comparable
//!     │       └── final class Puppy
//!     └── class Fish                       (not a Pet)
//!
//! class Point                              (equals / hash_code / to_string)
//! ```

use std::collections::HashSet;

use roost::prelude::*;

class! {
    public interface Named {
        fn name() -> String;
    }

    /// Something that can live with you.
    public interface Pet extends Named {
        fn owner() -> String;
        default fn greet() -> String {
            format!("{} greets {}", this.name(), this.owner())
        }
        default fn nickname() -> String {
            this.name().to_lowercase()
        }
        static fn species_count() -> i32 { 2 }
    }

    public interface Comparable {
        fn compare_to(other: Ref<Object>) -> i32;
    }

    public abstract class Animal implements Named {
        protected name: String;
        static created: i32 = 0;
        static registry: Vec<String>;
        private static secret: i32 = 42;

        protected Animal(name: String) {
            this.name = name;
            Animal::created += 1;
            let mut names = Self::registry;
            names.push(this.describe());
            Self::registry = names;
        }

        @Override public fn name() -> String { this.name }

        /// How many legs.
        public abstract fn legs() -> i32;
        public abstract fn sound() -> String;

        public fn describe() -> String {
            format!("{} ({} legs) says {}", this.name, this.legs(), this.sound())
        }

        public final fn kind() -> String { "animal".into() }

        public static fn created() -> i32 { Self::created }
        public static fn registry() -> Vec<String> { Self::registry }
        public static fn reset() {
            Self::created = 0;
            Self::registry = Vec::new();
        }
        public static fn secret() -> i32 { Animal::secret }
    }

    public class Dog extends Animal implements Pet, Comparable {
        private owner: String;

        public Dog(name: String, owner: String) {
            super(name);
            this.owner = owner;
        }

        @Override public fn legs() -> i32 { 4 }
        @Override public fn sound() -> String { "woof".into() }
        @Override public fn owner() -> String { this.owner }

        @Override
        public fn compare_to(other: Ref<Object>) -> i32 {
            match other.cast::<Dog>() {
                Some(d) => this.name().cmp(&d.name()) as i32,
                None => -1,
            }
        }

        // Inherited static method through the subclass, and an inherited static field.
        public static fn created_via_dog() -> i32 { Dog::created() + Self::created - Dog::created }
    }

    public final class Puppy extends Dog {
        public Puppy(name: String, owner: String) { super(name, owner); }

        @Override public fn sound() -> String { "yip".into() }
        @Override public fn greet() -> String { format!("{}!", super.greet()) }
    }

    public class Fish extends Animal {
        public Fish(name: String) { super(name); }
        @Override public fn legs() -> i32 { 0 }
        @Override public fn sound() -> String { "...".into() }
    }

    public class Point {
        x: i32;
        y: i32;

        public Point(x: i32, y: i32) { this.x = x; this.y = y; }

        public fn x() -> i32 { this.x }
        public fn y() -> i32 { this.y }

        @Override
        public fn equals(other: Ref<Object>) -> bool {
            match other.cast::<Point>() {
                Some(p) => this.x == p.x() && this.y == p.y(),
                None => false,
            }
        }

        @Override
        public fn hash_code() -> i32 { 31 * this.x + this.y }

        @Override
        public fn to_string() -> String { format!("Point({}, {})", this.x, this.y) }
    }

    public class Point3 extends Point {
        z: i32;
        public Point3(x: i32, y: i32, z: i32) { super(x, y); this.z = z; }

        @Override
        public fn to_string() -> String { format!("{} + z={}", super.to_string(), this.z) }
    }

    public class Plain {
        @Override
        public fn to_string() -> String { format!("plain/{}", super.to_string()) }
        @Override
        public fn equals(other: Ref<Object>) -> bool { super.equals(other) }
    }
}

#[test]
fn interface_references_are_polymorphic() {
    let pets: Vec<Ref<Pet>> = vec![Dog::new("Rex", "Ann"), Puppy::new("Bit", "Bob")];
    let greetings: Vec<String> = pets.iter().map(|p| p.greet()).collect();
    assert_eq!(greetings, ["Rex greets Ann", "Bit greets Bob!"]);
    // Methods of the super-interface, and a default method nobody overrides.
    assert_eq!(pets[1].name(), "Bit");
    assert_eq!(pets[1].nickname(), "bit");
    // Static interface method.
    assert_eq!(Pet::species_count(), 2);
    // Upcasts: interface -> super-interface -> Object.
    let named: Ref<Named> = pets[0].clone();
    let object: Ref<Object> = named.clone();
    assert_eq!(named.name(), "Rex");
    assert!(Ref::ptr_eq(&object, &pets[0]));
}

#[test]
fn a_class_reference_reaches_interface_methods() {
    let d: Ref<Dog> = Dog::new("Rex", "Ann");
    // `name` comes from Named (implemented in Animal), `greet` from Pet, `describe` from
    // Animal: all callable on a `Ref<Dog>`.
    assert_eq!(d.name(), "Rex");
    assert_eq!(d.greet(), "Rex greets Ann");
    assert_eq!(d.describe(), "Rex (4 legs) says woof");
    let p: Ref<Puppy> = Puppy::new("Bit", "Bob");
    assert_eq!(p.greet(), "Bit greets Bob!");
    assert_eq!(p.kind(), "animal");
}

#[test]
fn instanceof_and_cast_to_interfaces() {
    let a: Ref<Animal> = Puppy::new("Bit", "Bob");
    let f: Ref<Animal> = Fish::new("Nemo");
    assert!(a.instanceof::<Pet>());
    assert!(a.instanceof::<Named>());
    assert!(a.instanceof::<Comparable>());
    assert!(!f.instanceof::<Pet>());
    assert!(f.instanceof::<Named>(), "Animal implements Named");

    let pet: Ref<Pet> = a.cast::<Pet>().expect("a Puppy is a Pet");
    assert_eq!(pet.owner(), "Bob");
    assert!(Ref::ptr_eq(&pet, &a));
    assert!(f.cast::<Pet>().is_none());

    // From an interface back to a class, and across interfaces.
    let o: Ref<Object> = pet.clone();
    assert!(o.cast::<Puppy>().is_some());
    assert_eq!(o.cast::<Dog>().unwrap().sound(), "yip");
    let c: Ref<Comparable> = pet.cast::<Comparable>().unwrap();
    assert!(c.cast::<Pet>().is_some());
    assert_eq!(c.cast::<Animal>().unwrap().legs(), 4);
}

#[test]
fn interface_method_implemented_with_object_argument() {
    let rex = Dog::new("Rex", "Ann");
    let ace = Dog::new("Ace", "Ann");
    let c: Ref<Comparable> = rex.clone();
    assert_eq!(c.compare_to(ace.clone()), 1);
    assert_eq!(ace.compare_to(rex.clone()), -1);
    assert_eq!(rex.compare_to(rex.clone()), 0);
    assert_eq!(rex.compare_to(Point::new(1, 2)), -1);
}

#[test]
fn abstract_methods_dispatch_to_the_subclass() {
    let zoo: Vec<Ref<Animal>> = vec![
        Dog::new("Rex", "Ann"),
        Puppy::new("Bit", "Bob"),
        Fish::new("Nemo"),
    ];
    let described: Vec<String> = zoo.iter().map(|a| a.describe()).collect();
    assert_eq!(
        described,
        [
            "Rex (4 legs) says woof",
            "Bit (4 legs) says yip",
            "Nemo (0 legs) says ...",
        ]
    );
    assert_eq!(zoo[2].class_name(), "Fish");
}

#[test]
fn static_fields_are_shared_and_thread_local() {
    Animal::reset();
    Dog::new("Rex", "Ann");
    Fish::new("Nemo");
    // Written by Animal's constructor (as `Animal::created` and `Self::created`), read
    // through a static method and through the subclass.
    assert_eq!(Animal::created(), 2);
    assert_eq!(Dog::created_via_dog(), 2);
    assert_eq!(Animal::secret(), 42);
    // Constructors run with `this` complete, so the virtual calls see the subclass.
    assert_eq!(
        Animal::registry(),
        ["Rex (4 legs) says woof", "Nemo (0 legs) says ..."]
    );

    // Another thread has its own copy (and its own collector).
    let other = std::thread::spawn(|| {
        let before = Animal::created();
        Fish::new("Dory");
        (before, Animal::created())
    })
    .join()
    .unwrap();
    assert_eq!(other, (0, 1));
    assert_eq!(Animal::created(), 2);
}

#[test]
fn static_fields_can_hold_references() {
    class! {
        class Node {
            static last: Option<Ref<Node>>;
            value: i32;
            Node(value: i32) { this.value = value; }
            public fn value() -> i32 { this.value }
            public static fn remember(n: Ref<Node>) { Self::last = Some(n); }
            public static fn last() -> Option<Ref<Node>> { Self::last }
        }
    }
    Node::remember(Node::new(7));
    // The only reference is in the static: it is a root, so the object survives.
    roost::gc::collect();
    assert_eq!(Node::last().unwrap().value(), 7);
}

#[test]
fn equals_and_hash_code_overrides() {
    let a = Point::new(1, 2);
    let b = Point::new(1, 2);
    let c = Point::new(2, 1);
    // Value equality is `equals`, reference equality is `Ref::ptr_eq`.
    assert!(a.equals(b.clone()));
    assert!(!Ref::ptr_eq(&a, &b));
    assert!(!a.equals(c.clone()));
    assert!(!a.equals(Dog::new("Rex", "Ann")));
    assert_eq!(a.hash_code(), b.hash_code());
    assert_eq!(a.hash_code(), 33);
    // Through `Ref<Object>`: still the override.
    let o: Ref<Object> = a.clone();
    assert!(o.equals(b.clone()));
    assert_eq!(o.hash_code(), 33);
    // A subclass inherits them.
    let p3 = Point3::new(1, 2, 3);
    assert!(p3.equals(a.clone()));
    let hashes: HashSet<i32> = [a, b, c].iter().map(|p| p.hash_code()).collect();
    assert_eq!(hashes.len(), 2);
}

#[test]
fn to_string_override_and_display() {
    let p = Point::new(1, 2);
    assert_eq!(p.to_string(), "Point(1, 2)");
    assert_eq!(format!("{p}"), "Point(1, 2)");
    assert_eq!(format!("{p:?}"), "Ref(Point(1, 2))");
    let o: Ref<Object> = Point3::new(1, 2, 3);
    assert_eq!(format!("{o}"), "Point(1, 2) + z=3");
    // Default toString is `Class@hash`; `super.to_string()` reaches it.
    let plain = Plain::new();
    let expected = format!("plain/Plain@{:x}", plain.hash_code());
    assert_eq!(plain.to_string(), expected);
    assert!(plain.equals(plain.clone()));
    assert!(!plain.equals(Plain::new()));
    let d = Dog::new("Rex", "Ann");
    assert_eq!(d.to_string(), format!("Dog@{:x}", d.hash_code()));
}

#[test]
fn a_method_declared_by_a_superclass_and_by_interfaces() {
    // `title` is declared three times: by the class Book, and by two unrelated interfaces
    // that Novel implements (through Shelved). Book's method implements both, like Java.
    class! {
        interface Titled {
            fn title() -> String;
            default fn banner() -> String { format!("** {} **", this.title()) }
        }
        interface Labeled { fn title() -> String; }
        interface Shelved extends Titled, Labeled {}

        class Book {
            public fn title() -> String { "Dune".into() }
        }
        class Novel extends Book implements Shelved {
            public fn both() -> String { this.title() + " " + &this.banner() }
        }
        class Sequel extends Novel {
            @Override public fn title() -> String { super.title() + " Messiah" }
        }
    }
    let n: Ref<Novel> = Novel::new();
    assert_eq!(n.title(), "Dune");
    assert_eq!(n.both(), "Dune ** Dune **");
    let s: Ref<Shelved> = Sequel::new();
    assert_eq!(s.title(), "Dune Messiah");
    assert_eq!(s.banner(), "** Dune Messiah **");
    let l: Ref<Labeled> = s.cast::<Labeled>().unwrap();
    assert_eq!(l.title(), "Dune Messiah");
    let q: Ref<Sequel> = s.cast::<Sequel>().unwrap();
    assert_eq!(q.title(), "Dune Messiah");
    assert_eq!(q.both(), "Dune Messiah ** Dune Messiah **");
}
