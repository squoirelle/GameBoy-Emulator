mod bus;
mod cartridge;

use cartridge::Cartridge;
use std::path::Path;
use std::process;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Tetris.GB".to_string());

    let cart = match Cartridge::load(Path::new(&path)) {
        Ok(cart) => cart,
        Err(e) => {
            eprintln!("{path}: {e}");
            process::exit(1);
        }
    };

    println!("{}", cart.header);
}
