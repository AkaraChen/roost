//! `cargo run -p roost --example zoo`: the class DSL in one screen.
//!
//! Differences from the sketch in task #349:
//! * `let a: Ref<Animal> = Dog::new(..)` without `.into()`: the upcast is an implicit
//!   coercion (a `From` impl for it would overlap with core's `impl<T> From<T> for T`).
//!   In expression position there is `.upcast::<Animal>()`.
//! * `Puppy` declares a constructor: `Dog` has no no-arg constructor, so the implicit
//!   `super()` would not compile (in Java neither).
//! * A few getters (`legs`, `tricks`) so `main` can show the fields.

use roost::prelude::*;

class! {
    public class Animal {
        protected name: String;
        private legs: i32 = 4;

        public Animal(name: String) {
            this.name = name;
            this.init(); // virtual: lands in the subclass, like Java
        }

        public fn speak() -> String {
            format!("{} makes a sound", this.name)
        }

        public fn legs() -> i32 {
            this.legs
        }

        protected fn init() {}
    }

    public class Dog extends Animal {
        private tricks: i32;

        public Dog(name: String) {
            super(name);
        }

        @Override
        public fn speak() -> String {
            super.speak() + ", woof"
        }

        @Override
        protected fn init() {
            this.tricks = 1;
        }

        public fn tricks() -> i32 {
            this.tricks
        }

        public fn learn() {
            this.tricks += 1;
        }
    }

    public class Puppy extends Dog {
        public Puppy(name: String) {
            super(name);
        }

        @Override
        public fn speak() -> String {
            super.speak() + " (squeaky)"
        }
    }
}

fn main() {
    let a: Ref<Animal> = Dog::new("Rex".into());
    println!("a.speak()              = {}", a.speak());
    println!("a instanceof Dog       = {}", a.instanceof::<Dog>());

    let d: Option<Ref<Dog>> = a.cast::<Dog>();
    let d = d.expect("Rex is a Dog");
    println!("tricks (set in init()) = {}", d.tricks());
    d.learn();
    println!("tricks after learn()   = {}", d.tricks());
    println!("Ref::ptr_eq(&a, &d)    = {}", Ref::ptr_eq(&a, &d));

    let zoo: Vec<Ref<Animal>> = vec![
        Animal::new("Blob".into()),
        a.clone(),
        Puppy::new("Bit".into()).upcast(),
    ];
    for animal in &zoo {
        let as_dog = animal.cast::<Dog>();
        println!(
            "{:<6} speak = {:<38} legs = {}  as Dog: {}",
            animal.class_name(),
            animal.speak(),
            animal.legs(),
            match as_dog {
                Some(dog) => format!("tricks = {}", dog.tricks()),
                None => "None".into(),
            }
        );
    }
}
