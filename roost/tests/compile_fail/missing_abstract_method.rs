use roost::prelude::*;

class! {
    abstract class Shape {
        public abstract fn area() -> f64;
        public abstract fn name() -> String;
    }

    class Square extends Shape {
        @Override public fn name() -> String { "square".into() }
    }
}

fn main() {}
