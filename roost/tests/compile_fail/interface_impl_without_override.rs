use roost::prelude::*;

class! {
    interface Pet { fn name() -> String; }
    class Dog implements Pet {
        public fn name() -> String { "Rex".into() }
    }
}

fn main() {}
