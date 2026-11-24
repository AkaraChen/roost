use roost::prelude::*;

class! {
    public class Animal {
        public fn eat(grams: i32) -> String { format!("ate {grams}g") }
    }
    public class Dog extends Animal {
        @Override
        public fn eat(grams: f64) -> String { format!("gobbled {grams}g") }
    }
}

fn main() {}
