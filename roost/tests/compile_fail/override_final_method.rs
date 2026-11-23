use roost::prelude::*;

class! {
    class Animal {
        public final fn kind() -> String { "animal".into() }
    }
    class Dog extends Animal {
        @Override public fn kind() -> String { "dog".into() }
    }
}

fn main() {}
