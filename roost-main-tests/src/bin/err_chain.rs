//! An `Error` with a `source()`: the cause is printed as `Caused by:`.

use roost_main_tests::ConfigError;

#[roost::main]
fn main() -> Result<(), ConfigError> {
    let cause = "".parse::<i32>().unwrap_err();
    Err(ConfigError {
        file: "zoo.toml",
        cause,
    })
}
