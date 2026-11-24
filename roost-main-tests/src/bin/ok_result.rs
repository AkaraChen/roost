//! `main` returns `Ok(())`: exit code 0, nothing on stderr.

#[roost::main(gc_stats)]
fn main() -> Result<(), String> {
    println!("fine");
    Ok(())
}
