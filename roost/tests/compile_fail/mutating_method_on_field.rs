use roost::prelude::*;

class! {
    public class Kennel {
        private dogs: Vec<String>;
        public fn admit(name: String) { this.dogs.push(name); }
    }
}

fn main() {}
