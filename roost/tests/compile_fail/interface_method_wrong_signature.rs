use roost::prelude::*;

class! {
    interface Pet { fn age() -> i32; }
    class Dog implements Pet {
        @Override public fn age() -> String { "three".into() }
    }
}

fn main() {}
