use roost::prelude::*;

class! {
    class Counter {
        count: i32;
        static total: i32;
        public static fn get() -> i32 { this.count + Self::total }
    }
}

fn main() {}
