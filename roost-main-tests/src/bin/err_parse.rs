//! `main` returns `Err(ParseIntError)`: reported as an exception, exit code 1.

#[roost::main]
fn main() -> Result<(), std::num::ParseIntError> {
    let n: i32 = "12x".parse()?;
    println!("unreachable {n}");
    Ok(())
}
