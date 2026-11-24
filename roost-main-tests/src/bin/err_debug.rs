//! An error type with only `Debug`: printed with `{:?}`.

use roost_main_tests::Opaque;

#[roost::main]
fn main() -> Result<(), Opaque> {
    Err(Opaque(7))
}
