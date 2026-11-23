//! The two-level hierarchy the spike is about: `Animal` (root) -> `Dog`.
//!
//! What a user writes vs. what they get:
//! * `AnimalBase` — the root class's fields (also instantiable: a plain animal),
//! * `dyn Animal` — the vtable / reference type, used as `Gc<dyn Animal>`,
//! * `animal_super::*` — the base implementations, i.e. `super.method(...)`,
//! * `Dog` — embeds `AnimalBase` as its first field and overrides some vtable slots.

use gc::{Finalize, Gc, GcCell};

use crate::probe::{DropProbe, record_finalize};
use crate::{base_class, derived_class};

base_class! {
    pub class AnimalBase as dyn Animal {
        accessor: animal,
        upcast: as_animal,
        super_mod: animal_super,
        fields {
            pub name: GcCell<String>,
            pub friend: GcCell<Option<Gc<dyn Animal>>>,
            /// What `speak()` returned while the base constructor ran (see `AnimalBase::new`).
            pub first_words: String,
            probe: DropProbe,
        }
        methods {
            fn speak(&this) -> String {
                format!("{} makes a sound", this.animal().name.borrow())
            }

            // A base method that calls a virtual method: `this` here is the real object,
            // so `this.speak()` lands on Dog::speak for a dog.
            fn describe(&this) -> String {
                format!("[{}] {}", this.class_name(), this.speak())
            }

            fn greet_friend(&this) -> String {
                match &*this.animal().friend.borrow() {
                    Some(friend) => format!(
                        "{} greets {}: \"{}\"",
                        this.animal().name.borrow(),
                        friend.animal().name.borrow(),
                        friend.speak()
                    ),
                    None => format!("{} has no friend", this.animal().name.borrow()),
                }
            }

            // Hook for subclasses. The base does nothing.
            fn on_renamed(&this, _old: &str) -> String {
                String::new()
            }

            // Correct: release the borrow before calling out to (possibly overridden) code.
            fn rename(&this, new_name: String) -> String {
                let old = std::mem::replace(&mut *this.animal().name.borrow_mut(), new_name);
                this.on_renamed(&old)
            }

            // The natural-looking version. Fine for AnimalBase, panics for Dog, because
            // Dog::on_renamed reads `name` while this frame still holds `borrow_mut()`.
            fn rename_holding_borrow(&this, new_name: String) -> String {
                let mut name = this.animal().name.borrow_mut();
                let old = std::mem::replace(&mut *name, new_name);
                this.on_renamed(&old)
            }
        }
        gc_methods {
            // Needs `this` as a Gc to store it in the other object, hence `self: Gc<Self>`.
            fn befriend(this, other: Gc<dyn Animal>) {
                let me: Gc<dyn Animal> = this.as_animal();
                *me.animal().friend.borrow_mut() = Some(other.clone());
                *other.animal().friend.borrow_mut() = Some(me);
            }
        }
    }
}

impl AnimalBase {
    /// The "base constructor". Calls `speak()` on the half-built object to show the
    /// constructor trap: there is no `Gc<dyn Animal>` yet, only an `AnimalBase` value, so this
    /// is a static call to the *base* `speak`, even when building a Dog. (Java would dispatch
    /// to Dog.speak() here, with Dog's fields still null.)
    pub fn new(name: &str) -> AnimalBase {
        let mut base = AnimalBase {
            name: GcCell::new(name.to_owned()),
            friend: GcCell::new(None),
            first_words: String::new(),
            probe: DropProbe,
        };
        base.first_words = base.speak();
        base
    }

    pub fn create(name: &str) -> Gc<AnimalBase> {
        Gc::new(AnimalBase::new(name))
    }
}

// Counts for every animal, Dogs included: Dog's derived `finalize_glue` recurses into its
// `base: AnimalBase` field, so the base finalizer runs like a base-class destructor.
impl Finalize for AnimalBase {
    fn finalize(&self) {
        record_finalize();
    }
}

derived_class! {
    pub class Dog extends AnimalBase as dyn Animal {
        accessor: animal,
        upcast: as_animal,
        base_field: base,
        fields {
            pub breed: String,
        }
        overrides {
            fn speak(&this) -> String {
                // `super.speak()`
                format!("{} - Woof! (a {})", animal_super::speak(this), this.breed)
            }

            fn on_renamed(&this, old: &str) -> String {
                format!("{} now answers to {}", old, this.animal().name.borrow())
            }
        }
        gc_overrides {}
    }
}

impl Dog {
    pub fn create(name: &str, breed: &str) -> Gc<Dog> {
        Gc::new(Dog {
            base: AnimalBase::new(name),
            breed: breed.to_owned(),
        })
    }
}

impl Finalize for Dog {}
