use roost::prelude::*;

class! {
    class Animal {}
    interface Pet {}
    class Dog implements Animal {}
    class Cat extends Pet {}
}

fn main() {}
