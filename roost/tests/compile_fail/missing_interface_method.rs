use roost::prelude::*;

class! {
    interface Pet {
        fn name() -> String;
        fn owner() -> String;
        default fn greet() -> String { this.name() }
    }

    class Dog implements Pet {
        @Override public fn name() -> String { "Rex".into() }
    }
}

fn main() {}
