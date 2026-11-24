use roost::prelude::*;

class! {
    public class Animal {
        protected name: String;
        public Animal(name: String) { this.name = name; }
    }
    public class Dog extends Animal {
        public Dog() {}
    }
}

fn main() {}
