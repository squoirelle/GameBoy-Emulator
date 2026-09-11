mod bus;
mod cartridge;
mod registers;
mod cpu;

use crate::bus::Bus;
use crate::cpu::Cpu;
use cartridge::Cartridge;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::process;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Tetris.GB".to_string());

    // Tracing costs a formatted line and four bus reads per instruction, so it
    // stays off unless GB_TRACE names a file to write to.
    let mut trace = std::env::var("GB_TRACE").ok().map(|file| {
        BufWriter::new(File::create(&file).unwrap_or_else(|e| panic!("{file}: {e}")))
    });

    // The run loop is otherwise infinite and a trace grows without bound, so
    // GB_STEPS caps how many instructions we execute.
    let max_steps: u64 = std::env::var("GB_STEPS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(u64::MAX);

    let cart = match Cartridge::load(Path::new(&path)) {
        Ok(cart) => cart,
        Err(e) => {
            eprintln!("{path}: {e}");
            process::exit(1);
        }
    };

    let mut bus = Bus::new(cart);
    // A trace only exists to be fed to gameboy-doctor, which requires LY to
    // read as a constant 0x90, so tracing selects that mode too.
    bus.set_ly_fixed(trace.is_some());

    let mut cpu = Cpu::new();

    let mut steps = 0u64;
    while steps < max_steps {
        if let Some(out) = trace.as_mut() {
            cpu.write_trace(&bus, out).expect("failed to write the trace");
        }
        cpu.step(&mut bus);
        steps += 1;
    }

    // BufWriter flushes when dropped but swallows the error, so do it here.
    if let Some(out) = trace.as_mut() {
        out.flush().expect("failed to flush the trace");
    }
}
