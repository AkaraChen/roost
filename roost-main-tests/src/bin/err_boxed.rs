//! The common `Box<dyn Error>`, here with a cause behind it.

use roost_main_tests::ConfigError;

#[roost::main]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cause = "x".parse::<i32>().unwrap_err();
    Err(Box::new(ConfigError {
        file: "zoo.toml",
        cause,
    }))
}
