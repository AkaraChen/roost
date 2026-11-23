use roost::prelude::*;

class! {
    public class Animal {}
    public class Pet {}
    public class Dog extends Animal, Pet {}
}

fn main() {}
