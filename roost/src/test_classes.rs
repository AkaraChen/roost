//! A small hierarchy for the unit tests, written by hand the way `class!` is expected to
//! generate it:
//!
//! ```text
//! Object
//! └── Animal          trait Animal,  instantiable as PlainAnimal
//!     ├── Dog         trait Dog,     instantiable as Mutt       (non-leaf)
//!     │   └── Puppy                  leaf
//!     └── Cat                        leaf, overrides toString
//! ```

use crate::gc::{Finalize, GcCell, Probe, Trace};
use crate::{CastRequest, Object, ObjectClass, Ref};

pub trait Animal: ObjectClass {
    fn name(&self) -> String;

    fn speak(&self) -> String {
        format!("{} makes a sound", self.name())
    }

    /// A `self: Ref<Self>` receiver, dispatched on `Ref<dyn Animal>` (`DispatchFromDyn`).
    fn name_via_ref(self: Ref<Self>) -> String {
        self.name()
    }
}

pub trait Dog: Animal {
    fn fetch(&self) -> String {
        format!("{} fetches", self.name())
    }
}

#[derive(Trace, Finalize)]
pub struct PlainAnimal {
    pub name: String,
    pub friend: GcCell<Option<Ref<dyn Animal>>>,
    pub probe: Option<Probe>,
}

#[derive(Trace, Finalize)]
pub struct Mutt {
    pub name: String,
}

#[derive(Trace, Finalize)]
pub struct Puppy {
    pub name: String,
    pub probe: Option<Probe>,
}

#[derive(Trace, Finalize)]
pub struct Cat {
    pub name: String,
}

pub fn plain(name: &str) -> Ref<PlainAnimal> {
    Ref::new(PlainAnimal {
        name: name.into(),
        friend: GcCell::new(None),
        probe: None,
    })
}

pub fn mutt(name: &str) -> Ref<Mutt> {
    Ref::new(Mutt { name: name.into() })
}

pub fn puppy(name: &str) -> Ref<Puppy> {
    Ref::new(Puppy {
        name: name.into(),
        probe: None,
    })
}

pub fn cat(name: &str) -> Ref<Cat> {
    Ref::new(Cat { name: name.into() })
}

impl ObjectClass for PlainAnimal {
    fn __roost_casts(&self, req: &mut CastRequest<'_>) {
        req.offer_class(self)
            .offer::<dyn Animal, _>(self)
            .offer::<Object, _>(self);
    }
}
impl Animal for PlainAnimal {
    fn name(&self) -> String {
        self.name.clone()
    }
}

impl ObjectClass for Mutt {
    fn __roost_casts(&self, req: &mut CastRequest<'_>) {
        req.offer_class(self)
            .offer::<dyn Dog, _>(self)
            .offer::<dyn Animal, _>(self)
            .offer::<Object, _>(self);
    }
}
impl Animal for Mutt {
    fn name(&self) -> String {
        self.name.clone()
    }
    fn speak(&self) -> String {
        format!("{} says woof", self.name())
    }
}
impl Dog for Mutt {}

impl ObjectClass for Puppy {
    fn __roost_casts(&self, req: &mut CastRequest<'_>) {
        req.offer_class(self)
            .offer::<dyn Dog, _>(self)
            .offer::<dyn Animal, _>(self)
            .offer::<Object, _>(self);
    }
}
impl Animal for Puppy {
    fn name(&self) -> String {
        self.name.clone()
    }
    fn speak(&self) -> String {
        format!("{} says yip", self.name())
    }
}
impl Dog for Puppy {
    fn fetch(&self) -> String {
        format!("{} fetches clumsily", self.name())
    }
}

impl ObjectClass for Cat {
    fn __roost_casts(&self, req: &mut CastRequest<'_>) {
        req.offer_class(self)
            .offer::<dyn Animal, _>(self)
            .offer::<Object, _>(self);
    }
    fn class_name(&self) -> &'static str {
        "Cat"
    }
    fn to_string(&self) -> String {
        format!("Cat({})", self.name)
    }
}
impl Animal for Cat {
    fn name(&self) -> String {
        self.name.clone()
    }
}
