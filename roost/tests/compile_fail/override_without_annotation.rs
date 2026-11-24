use roost::prelude::*;

class! {
    public class Animal {
        public fn speak() -> String { "...".to_string() }
    }
    public class Dog extends Animal {
        public fn speak() -> String { "woof".to_string() }
    }
}

fn main() {}
