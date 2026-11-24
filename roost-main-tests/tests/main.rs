//! `#[roost::main]`, observed from outside: each fixture in `src/bin/` is run as a
//! subprocess and its exit code, stdout and stderr are checked.

use std::process::{Command, Output};

fn run(exe: &str) -> Output {
    Command::new(exe)
        // Keep the report independent of the developer's environment.
        .env_remove("RUST_BACKTRACE")
        .env_remove("RUST_LIB_BACKTRACE")
        .output()
        .unwrap_or_else(|e| panic!("cannot run {exe}: {e}"))
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Runs `exe`, checks its exit code, returns (stdout, stderr).
fn run_expecting(exe: &str, code: i32) -> (String, String) {
    let out = run(exe);
    let (stdout, stderr) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(
        out.status.code(),
        Some(code),
        "{exe}: exit status {:?}\n--- stdout\n{stdout}\n--- stderr\n{stderr}",
        out.status
    );
    (stdout, stderr)
}

#[test]
fn stack_argument_sizes_the_main_thread() {
    // ~20 MiB of recursion (more in debug builds): fine on a 256 MB stack...
    let (stdout, _) = run_expecting(env!("CARGO_BIN_EXE_stack_big"), 0);
    assert!(stdout.starts_with("recursed 5000 frames"), "{stdout}");

    // ...and an overflow on a 2 MB one, so it is the argument that makes the difference.
    let out = run(env!("CARGO_BIN_EXE_stack_small"));
    let stderr = text(&out.stderr);
    assert!(!out.status.success(), "{stderr}");
    assert!(stderr.contains("overflowed its stack"), "{stderr}");
}

#[test]
fn final_collection_runs_after_main_returns() {
    let (stdout, stderr) = run_expecting(env!("CARGO_BIN_EXE_gc_exit"), 0);
    // The cycle is only finalized by a collection, after `main` returned...
    assert_eq!(
        stdout.lines().collect::<Vec<_>>(),
        ["main returns", "finalized a Node", "finalized a Node"],
    );
    // ...and before the statistics are printed, which the shell does right after its
    // `collect()` (the thread-local heap's own teardown would come later still).
    assert!(
        stderr.contains("[roost gc] objects allocated: 2, collections: 1"),
        "{stderr}"
    );
    assert!(stderr.contains("left: 0 bytes"), "{stderr}");
}

#[test]
fn borrow_conflict_is_reported_like_a_java_exception() {
    let (stdout, stderr) = run_expecting(env!("CARGO_BIN_EXE_borrow_conflict"), 1);
    assert_eq!(stdout, "");
    let mut lines = stderr.lines();
    let first = lines.next().unwrap_or_default();
    // The field is inherited: the class named is the one that declares it.
    assert!(
        first.starts_with(
            "Exception in thread \"main\" roost.BorrowConflict: field Animal.name cannot be \
             written while it is being read"
        ),
        "{stderr}"
    );
    // Where: the user's `this.name = Name` (in `rename`), not roost's internals.
    let at = lines.next().unwrap_or_default();
    assert!(
        at.starts_with("\tat ") && at.contains("src/bin/borrow_conflict.rs:"),
        "{stderr}"
    );
    assert!(!stderr.contains("panicked at"), "{stderr}");
}

#[test]
fn other_panics_keep_the_default_report() {
    let (_, stderr) = run_expecting(env!("CARGO_BIN_EXE_plain_panic"), 101);
    assert!(stderr.contains("thread 'main'"), "{stderr}");
    assert!(stderr.contains("panicked at"), "{stderr}");
    assert!(stderr.contains("index out of bounds"), "{stderr}");
    assert!(!stderr.contains("Exception in thread"), "{stderr}");
}

#[test]
fn err_from_main_exits_with_1() {
    let (_, stderr) = run_expecting(env!("CARGO_BIN_EXE_err_parse"), 1);
    assert_eq!(
        stderr,
        "Exception in thread \"main\" ParseIntError: invalid digit found in string\n"
    );

    let (_, stderr) = run_expecting(env!("CARGO_BIN_EXE_err_chain"), 1);
    assert_eq!(
        stderr,
        "Exception in thread \"main\" ConfigError: bad config file zoo.toml\n\
         Caused by: cannot parse integer from empty string\n"
    );

    let (_, stderr) = run_expecting(env!("CARGO_BIN_EXE_err_boxed"), 1);
    assert_eq!(
        stderr,
        "Exception in thread \"main\" Box<dyn Error>: bad config file zoo.toml\n\
         Caused by: invalid digit found in string\n"
    );

    let (_, stderr) = run_expecting(env!("CARGO_BIN_EXE_err_debug"), 1);
    assert_eq!(stderr, "Exception in thread \"main\" Opaque: Opaque(7)\n");
}

#[test]
fn ok_from_main_exits_with_0() {
    let (stdout, stderr) = run_expecting(env!("CARGO_BIN_EXE_ok_result"), 0);
    assert_eq!(stdout, "fine\n");
    assert!(stderr.starts_with("[roost gc] "), "{stderr}");
}

#[test]
fn misuse_is_a_compile_error() {
    trybuild::TestCases::new().compile_fail("tests/ui/*.rs");
}
