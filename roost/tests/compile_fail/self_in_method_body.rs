use roost::prelude::*;

class! {
    public class Dog {
        private name: String;
        public fn name() -> String { self.name }
    }
}

fn main() {}
