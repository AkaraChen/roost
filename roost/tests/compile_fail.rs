//! Compile-time errors of `class!`, with their exact messages (`tests/compile_fail/*.stderr`).
//!
//! The snapshots depend on the compiler: they are for the nightly pinned in
//! `rust-toolchain.toml`. After bumping it, regenerate with
//! `TRYBUILD=overwrite cargo test -p roost --test compile_fail` and review the diff.

#[test]
fn compile_fail() {
    trybuild::TestCases::new().compile_fail("tests/compile_fail/*.rs");
}
