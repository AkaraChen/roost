//! Deep recursion on a 256 MB stack: must succeed.

#[roost::main(stack = "256MB")]
fn main() {
    let r = roost_main_tests::recurse(roost_main_tests::DEPTH);
    println!("recursed {} frames ({r})", roost_main_tests::DEPTH);
}
