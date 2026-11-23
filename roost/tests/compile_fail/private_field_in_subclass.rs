use roost::prelude::*;

class! {
    public class Animal {
        private name: String;
    }
    public class Dog extends Animal {
        public fn name() -> String { this.name }
    }
}

fn main() {}
