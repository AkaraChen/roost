use roost::prelude::*;

class! {
    public class Dog {
        private friend: Option<Ref<Dog>>;
        public fn forget() { this.friend = null; }
    }
}

fn main() {}
