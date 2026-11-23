//! Helpers shared by the fixture programs in `src/bin/`. Each fixture is a small program
//! using `#[roost::main]`; `tests/main.rs` runs them and checks their output and exit code.

use std::hint::black_box;

/// Recursion with a 4 KiB frame: `recurse(n)` needs about `4 * n` KiB of stack.
#[inline(never)]
pub fn recurse(n: u64) -> u64 {
    let frame = black_box([n as u8; 4096]);
    if n == 0 {
        return 0;
    }
    recurse(n - 1) + u64::from(black_box(frame)[n as usize % 4096])
}

/// The recursion depth the stack fixtures use: at least 20 MiB (more in debug builds), well over the 8 MiB a plain
/// `fn main` gets on Linux.
pub const DEPTH: u64 = 5_000;

/// An error with a `source()`, for the `Caused by:` chain.
#[derive(Debug)]
pub struct ConfigError {
    pub file: &'static str,
    pub cause: std::num::ParseIntError,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "bad config file {}", self.file)
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}

/// Only `Debug`, no `Display`.
#[derive(Debug)]
pub struct Opaque(pub i32);
