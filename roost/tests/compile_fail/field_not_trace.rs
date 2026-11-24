use roost::prelude::*;

struct Collar {
    size: u32,
}

class! {
    public class Dog {
        private collar: Option<Collar>;
    }
}

fn main() {}
