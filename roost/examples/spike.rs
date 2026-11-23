//! `cargo run --example spike`: walks through the five things the first spike demonstrates.
//!
//! Shares its object model with `tests/spike.rs` (modules in `tests/spike/`).
#![feature(arbitrary_self_types)]

#[path = "../tests/spike/object.rs"]
#[allow(dead_code)]
mod object;
#[path = "../tests/spike/probe.rs"]
mod probe;
#[path = "../tests/spike/zoo.rs"]
#[allow(dead_code)]
mod zoo;

use gc::Gc;
use object::GcExt;
use zoo::{Animal, AnimalBase, Dog, animal_super};

fn main() {
    println!("== 1. inheritance: Dog embeds AnimalBase, referenced as Gc<dyn Animal>");
    let rex: Gc<dyn Animal> = Dog::create("Rex", "beagle");
    let blob: Gc<dyn Animal> = AnimalBase::create("Blob");
    println!(
        "   rex.animal().name = {:?}, size_of::<Gc<dyn Animal>>() = {} bytes (data ptr + vtable ptr)",
        rex.animal().name.borrow(),
        size_of::<Gc<dyn Animal>>()
    );

    println!("== 2. override + dynamic dispatch through the vtable");
    for a in [&rex, &blob] {
        println!(
            "   {:<10} speak()    -> {}",
            a.class_name(),
            speak_via_vtable(a)
        );
        println!("   {:<10} describe() -> {}", a.class_name(), a.describe());
    }

    println!("== 3. super call");
    println!(
        "   animal_super::speak(rex) -> {}",
        animal_super::speak(&*rex)
    );
    println!(
        "   speak() called in the base constructor of rex -> {:?} (base version: no vtable yet)",
        rex.animal().first_words
    );

    println!("== 4. instanceof / downcast");
    println!(
        "   rex.is::<Dog>() = {}, rex.is::<AnimalBase>() = {}, blob.is::<Dog>() = {}",
        rex.is::<Dog>(),
        rex.is::<AnimalBase>(),
        blob.is::<Dog>()
    );
    match rex.cast::<Dog>() {
        Some(dog) => println!("   rex.cast::<Dog>()  -> Some(breed = {:?})", dog.breed),
        None => println!("   rex.cast::<Dog>()  -> None"),
    }
    println!(
        "   blob.cast::<Dog>() -> {}",
        if blob.cast::<Dog>().is_some() {
            "Some"
        } else {
            "None"
        }
    );
    let rex2 = rex.clone();
    println!(
        "   Gc::ptr_eq(rex, rex.clone()) = {}, Gc::ptr_eq(rex, blob) = {}",
        Gc::ptr_eq(&rex, &rex2),
        Gc::ptr_eq(&rex, &blob)
    );
    drop(rex2);

    println!("== 5. cycle collection");
    rex.clone().befriend(blob.clone());
    println!("   {}", rex.greet_friend());
    println!("   {}", blob.greet_friend());
    drop(rex);
    drop(blob);
    println!(
        "   handles dropped; before collect: dropped = {}, finalized = {}",
        probe::dropped(),
        probe::finalized()
    );
    roost::gc::collect();
    println!(
        "   after roost::gc::collect():      dropped = {}, finalized = {}",
        probe::dropped(),
        probe::finalized()
    );
}

/// Non-generic: the call inside can only be resolved through the vtable.
#[inline(never)]
fn speak_via_vtable(animal: &Gc<dyn Animal>) -> String {
    animal.speak()
}
