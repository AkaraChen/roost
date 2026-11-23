//! The same recursion as `stack_big` on a 2 MB stack: must overflow. Shows that the
//! `stack` argument is what makes `stack_big` pass.

#[roost::main(stack = "2MB")]
fn main() {
    let r = roost_main_tests::recurse(roost_main_tests::DEPTH);
    println!("recursed {} frames ({r})", roost_main_tests::DEPTH);
}
