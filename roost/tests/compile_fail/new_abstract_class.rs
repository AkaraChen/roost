use roost::prelude::*;

class! {
    abstract class Shape {
        public abstract fn area() -> f64;
    }
}

fn main() {
    let _s: Ref<Shape> = Shape::new();
}
