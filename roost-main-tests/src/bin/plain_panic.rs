//! A panic that is not a roost exception: left to Rust's own hook, exit code 101.

#[roost::main]
fn main() {
    let v: Vec<i32> = Vec::new();
    println!("{}", v[3]);
}
