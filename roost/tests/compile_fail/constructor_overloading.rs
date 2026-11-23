use roost::prelude::*;

class! {
    public class Dog {
        private name: String;
        public Dog() { this.name = "Rex".to_string(); }
        public Dog(name: String) { this.name = name; }
    }
}

fn main() {}
