use roost::prelude::*;

class! {
    interface Titled { fn title() -> String; }
    class Book { public fn title() -> i32 { 1 } }
    class Novel extends Book implements Titled {}
}

fn main() {}
