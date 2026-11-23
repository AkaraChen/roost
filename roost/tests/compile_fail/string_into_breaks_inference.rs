use roost::prelude::*;

class! {
    public class Dog {
        private name: String;
        public Dog(name: String) { this.name = name; }
    }
}

fn main() {
    let _d = Dog::new("Rex".into());
}
