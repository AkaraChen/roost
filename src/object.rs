//! The runtime half of the object model: class identity, `is` / `cast`, and the two
//! `macro_rules!` macros that turn a class description into structs, traits and impls.

use std::any::TypeId;

use gc::{Gc, Trace};

/// Root of every class hierarchy. Every class trait (`Animal`, ...) has it as a supertrait,
/// so these methods live in every vtable.
pub trait Object: Trace + 'static {
    /// Exact runtime class of this object.
    #[doc(hidden)]
    fn __class_id(&self) -> TypeId;

    /// `instanceof`: is the runtime class `class` or one of its subclasses?
    #[doc(hidden)]
    fn __instance_of(&self, class: TypeId) -> bool;

    fn class_name(&self) -> &'static str;
}

/// Static (per concrete class) information.
pub trait Class: Object + Sized {
    const NAME: &'static str;

    /// Is `Self` the class `class`, or a subclass of it?
    #[doc(hidden)]
    fn __is_subclass_of(class: TypeId) -> bool;
}

/// `instanceof` and checked downcasts on any `Gc<dyn SomeClassTrait>`.
pub trait GcExt {
    /// Java `x instanceof T`. True for `T`'s subclasses too.
    fn is<T: Class>(&self) -> bool;

    /// Checked downcast to the *exact* runtime class. `None` on mismatch, never panics.
    ///
    /// Note: `dog.is::<AnimalBase>()` is true but `dog.cast::<AnimalBase>()` is `None`:
    /// with field-containment inheritance a `Gc<AnimalBase>` cannot point into the middle of
    /// a Dog's allocation. Upcasting goes through unsizing (`Gc<Dog>` -> `Gc<dyn Animal>`).
    fn cast<T: Class>(&self) -> Option<Gc<T>>;
}

impl<U: Object + ?Sized> GcExt for Gc<U> {
    fn is<T: Class>(&self) -> bool {
        (**self).__instance_of(TypeId::of::<T>())
    }

    fn cast<T: Class>(&self) -> Option<Gc<T>> {
        if (**self).__class_id() != TypeId::of::<T>() {
            return None;
        }
        // The allocation was created as a `GcBox<T>` (by `Gc::<T>::new`) and later unsized.
        // `into_raw` keeps the root that `clone` added; `from_raw` takes it over, exactly like
        // `Rc::into_raw` / `Rc::from_raw`. Dropping the metadata with `.cast()` is fine because
        // `T` is the exact runtime type, so `Layout::for_value` inside `from_raw` finds the
        // same `GcBox<T>` layout the allocation was made with.
        let raw: *const U = Gc::into_raw(self.clone());
        Some(unsafe { Gc::from_raw(raw.cast::<T>()) })
    }
}

/// Declares a root class.
///
/// Generates:
/// * `struct $Base` holding the fields (derives `Trace`; you write `Finalize`),
/// * `trait $Trait: Object` — the vtable. Every virtual method has a default body that
///   forwards to the "super" implementation,
/// * `mod $super_mod` with one free function per virtual method — the base implementation,
///   generic over `T: ?Sized + $Trait`, so `self` inside it is the *real* object and calls on
///   it are still virtual. Overrides call `$super_mod::speak(this)` for `super.speak()`,
/// * `impl $Trait for $Base` (the base class is concrete) plus `Object` / `Class`.
///
/// Method bodies name the receiver `this` instead of `self`: the base bodies become generic
/// free functions (`fn speak<T: ?Sized + Animal>(this: &T)`), and `self` can't be a free
/// function's parameter. Overrides use `this` too, for symmetry.
///
/// Methods come in two sections because a `macro_rules!` repetition can't choose between
/// receiver shapes: `methods` are `fn m(&this, ..)` and become `&self` vtable methods;
/// `gc_methods` are `fn m(this, ..)` and become `self: Gc<Self>` vtable methods.
#[macro_export]
macro_rules! base_class {
    (
        $(#[$meta:meta])*
        $vis:vis class $Base:ident as dyn $Trait:ident {
            accessor: $accessor:ident,
            upcast: $upcast:ident,
            super_mod: $super_mod:ident,
            fields { $( $(#[$fmeta:meta])* $fvis:vis $field:ident : $fty:ty ),* $(,)? }
            methods {
                $(
                    fn $m:ident (&$mthis:ident $(, $marg:ident : $maty:ty)* $(,)?) $(-> $mret:ty)?
                        $mbody:block
                )*
            }
            gc_methods {
                $(
                    fn $g:ident ($gthis:ident $(, $garg:ident : $gaty:ty)* $(,)?) $(-> $gret:ty)?
                        $gbody:block
                )*
            }
        }
    ) => {
        $(#[$meta])*
        #[derive(::gc::Trace)]
        $vis struct $Base {
            $( $(#[$fmeta])* $fvis $field : $fty, )*
        }

        $vis trait $Trait: $crate::object::Object {
            /// Field access through the vtable: the embedded base-class fields.
            fn $accessor(&self) -> &$Base;

            /// `this` as a GC pointer to the class trait object. Has to be generated per
            /// class: in a default method `Self: ?Sized`, so `Gc<Self>` can't be unsized.
            fn $upcast(self: ::gc::Gc<Self>) -> ::gc::Gc<dyn $Trait>;

            $(
                fn $m(&self $(, $marg : $maty)*) $(-> $mret)? {
                    $super_mod::$m(self $(, $marg)*)
                }
            )*

            $(
                fn $g(self: ::gc::Gc<Self> $(, $garg : $gaty)*) $(-> $gret)? {
                    $super_mod::$g(self $(, $garg)*)
                }
            )*
        }

        /// Base-class implementations, callable as `super.method()` from overrides.
        ///
        /// Generic over `T: ?Sized + Trait`, so `this.speak()` inside a base body is still
        /// a call through the trait (virtual when `T = dyn Trait`).
        #[allow(dead_code)]
        $vis mod $super_mod {
            #[allow(unused_imports)]
            use super::*;

            $(
                #[allow(unused_variables)]
                pub fn $m<T: ?Sized + $Trait>($mthis: &T $(, $marg : $maty)*) $(-> $mret)?
                    $mbody
            )*
            $(
                #[allow(unused_variables)]
                pub fn $g<T: ?Sized + $Trait>($gthis: ::gc::Gc<T> $(, $garg : $gaty)*) $(-> $gret)?
                    $gbody
            )*
        }

        impl $Trait for $Base {
            fn $accessor(&self) -> &$Base {
                self
            }
            fn $upcast(self: ::gc::Gc<Self>) -> ::gc::Gc<dyn $Trait> {
                self
            }
        }

        $crate::__object_impls!($Base);
    };
}

/// Declares a class deriving from a root class (one level; see README for why).
///
/// The base-class part is embedded as the field `$base_field: $Base` (field containment).
/// `override` methods replace the vtable entry; anything not overridden keeps the default
/// (= base implementation).
#[macro_export]
macro_rules! derived_class {
    (
        $(#[$meta:meta])*
        $vis:vis class $Name:ident extends $Base:ident as dyn $Trait:ident {
            accessor: $accessor:ident,
            upcast: $upcast:ident,
            base_field: $base_field:ident,
            fields { $( $(#[$fmeta:meta])* $fvis:vis $field:ident : $fty:ty ),* $(,)? }
            overrides {
                $(
                    fn $m:ident (&$mthis:ident $(, $marg:ident : $maty:ty)* $(,)?) $(-> $mret:ty)?
                        $mbody:block
                )*
            }
            gc_overrides {
                $(
                    fn $g:ident ($gthis:ident $(, $garg:ident : $gaty:ty)* $(,)?) $(-> $gret:ty)?
                        $gbody:block
                )*
            }
        }
    ) => {
        $(#[$meta])*
        #[derive(::gc::Trace)]
        $vis struct $Name {
            pub $base_field: $Base,
            $( $(#[$fmeta])* $fvis $field : $fty, )*
        }

        impl $Trait for $Name {
            fn $accessor(&self) -> &$Base {
                &self.$base_field
            }
            fn $upcast(self: ::gc::Gc<Self>) -> ::gc::Gc<dyn $Trait> {
                self
            }
            $(
                fn $m(&self $(, $marg : $maty)*) $(-> $mret)? {
                    let $mthis = self;
                    $mbody
                }
            )*
            $(
                fn $g(self: ::gc::Gc<Self> $(, $garg : $gaty)*) $(-> $gret)? {
                    let $gthis = self;
                    $gbody
                }
            )*
        }

        $crate::__object_impls!($Name extends $Base);
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __object_impls {
    ($Name:ident $(extends $Parent:ident)?) => {
        impl $crate::object::Object for $Name {
            fn __class_id(&self) -> ::std::any::TypeId {
                ::std::any::TypeId::of::<$Name>()
            }
            fn __instance_of(&self, class: ::std::any::TypeId) -> bool {
                <$Name as $crate::object::Class>::__is_subclass_of(class)
            }
            fn class_name(&self) -> &'static str {
                <$Name as $crate::object::Class>::NAME
            }
        }

        impl $crate::object::Class for $Name {
            const NAME: &'static str = stringify!($Name);
            fn __is_subclass_of(class: ::std::any::TypeId) -> bool {
                class == ::std::any::TypeId::of::<$Name>()
                    $( || <$Parent as $crate::object::Class>::__is_subclass_of(class) )?
            }
        }
    };
}
