//! roost spike: GC + class inheritance + virtual dispatch.
//!
//! Not a library. See README.md for what this proves and what hurt.

// `Gc<dyn Trait>` itself needs gc's `nightly` feature (CoerceUnsized / DispatchFromDyn on Gc).
// On our side we additionally need `arbitrary_self_types` so a virtual method can take
// `self: Gc<Self>` (the only way for a method to get hold of `this` as a GC pointer).
#![feature(arbitrary_self_types)]
// gc_derive 0.5.0 wraps its impls in `const _DERIVE_...: () = { impl ... }`, which trips
// this lint on every `#[derive(Trace)]` on a modern compiler. Nothing we can fix locally.
#![allow(non_local_definitions)]

pub mod object;
pub mod probe;
pub mod zoo;

pub use gc::{Finalize, Gc, GcCell, Trace, force_collect};
pub use object::{Class, GcExt, Object};
