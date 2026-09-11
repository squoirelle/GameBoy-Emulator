mod bus;
mod cartridge;
mod cpu;
mod joypad;
mod ppu;
mod registers;
mod timer;

use crate::bus::Bus;
use crate::cpu::Cpu;
use crate::joypad::Button;
use cartridge::Cartridge;
use minifb::{Key, Window, WindowOptions};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::process;

/// The four DMG shades as 0x00RRGGBB, lightest first, indexed by the 2-bit
/// value BGP produced.
const PALETTE: [u32; 4] = [0x00E0F8D0, 0x0088C070, 0x00346856, 0x00081820];

const WIDTH: usize = 160;
const HEIGHT: usize = 144;

const KEYMAP: [(Key, Button); 8] = [
    (Key::Right, Button::Right),
    (Key::Left, Button::Left),
    (Key::Up, Button::Up),
    (Key::Down, Button::Down),
    (Key::Z, Button::A),
    (Key::X, Button::B),
    (Key::Backspace, Button::Select),
    (Key::Enter, Button::Start),
];

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Tetris.GB".to_string());

    // Tracing costs a formatted line and four bus reads per instruction, so it
    // stays off unless GB_TRACE names a file to write to.
    let mut trace = std::env::var("GB_TRACE").ok().map(|file| {
        BufWriter::new(File::create(&file).unwrap_or_else(|e| panic!("{file}: {e}")))
    });

    // GB_STEPS both caps the run and selects the headless path: executing a
    // fixed number of instructions only makes sense without a window.
    let max_steps: Option<u64> = std::env::var("GB_STEPS")
        .ok()
        .and_then(|s| s.parse().ok());

    // gameboy-doctor's reference logs were made with LY pinned to 0x90, so any
    // run being compared against them needs the same.
    let ly_fixed: bool = std::env::var("GB_DOCTOR")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(false);

    let cart = match Cartridge::load(Path::new(&path)) {
        Ok(cart) => cart,
        Err(e) => {
            eprintln!("{path}: {e}");
            process::exit(1);
        }
    };
    println!("{}", cart.header);

    let mut bus = Bus::new(cart);
    bus.set_ly_fixed(ly_fixed);
    let mut cpu = Cpu::new();

    match max_steps {
        Some(steps) => run_headless(&mut cpu, &mut bus, steps, &mut trace),
        None => run_windowed(&mut cpu, &mut bus, &mut trace),
    }

    // BufWriter flushes when dropped but swallows the error, so do it here.
    if let Some(out) = trace.as_mut() {
        out.flush().expect("failed to flush the trace");
    }
}

/// Runs a fixed number of instructions and stops, with nothing on screen. Test
/// ROMs report over the serial port and gameboy-doctor reads the trace file, so
/// neither has any use for a window.
fn run_headless<W: Write>(cpu: &mut Cpu, bus: &mut Bus, max_steps: u64, trace: &mut Option<W>) {
    for _ in 0..max_steps {
        step_once(cpu, bus, trace);
    }
    if std::env::var("GB_ASCII").is_ok() {
        dump_framebuffer(bus);
    }
}

/// Prints the last rendered frame as text, one character per pixel. Lets the
/// renderer be checked without a window in the way.
fn dump_framebuffer(bus: &Bus) {
    let p = bus.ppu();
    let tiles = p.vram[0x0000..0x1800].iter().filter(|&&b| b != 0).count();
    let map0 = p.vram[0x1800..0x1C00].iter().filter(|&&b| b != 0).count();
    let map1 = p.vram[0x1C00..0x2000].iter().filter(|&&b| b != 0).count();
    eprintln!(
        "LCDC:{:02X} BGP:{:02X} SCX:{:02X} SCY:{:02X} LY:{:02X} | tiles:{} map9800:{} map9C00:{} oam:{}",
        p.lcdc,
        p.bgp,
        p.scx,
        p.scy,
        p.ly,
        tiles,
        map0,
        map1,
        bus.oam().iter().filter(|&&b| b != 0).count()
    );

    const SHADES: [char; 4] = [' ', '.', ':', '#'];
    for row in bus.framebuffer().chunks(WIDTH) {
        let line: String = row.iter().map(|&s| SHADES[s as usize & 3]).collect();
        println!("|{line}|");
    }
}

/// Runs until the window closes, one frame at a time.
fn run_windowed<W: Write>(cpu: &mut Cpu, bus: &mut Bus, trace: &mut Option<W>) {
    let mut buffer = vec![0u32; WIDTH * HEIGHT];

    let mut window = Window::new("Game Boy", WIDTH, HEIGHT, WindowOptions::default())
        .unwrap_or_else(|e| panic!("could not open a window: {e}"));
    // update_with_buffer blocks until the frame is due, which is the whole of
    // the frame pacing this needs.
    window.set_target_fps(60);

    while window.is_open() && !window.is_key_down(Key::Escape) {
        // Sampled once a frame. Games poll the register far more often than
        // that, but nobody presses a key for less than 16 milliseconds.
        for (key, button) in KEYMAP {
            bus.set_button(button, window.is_key_down(key));
        }

        // However many instructions fit in a frame. The count varies, so the
        // PPU decides when to stop rather than us counting.
        while !step_once(cpu, bus, trace) {}

        for (shade, out) in bus.framebuffer().iter().zip(buffer.iter_mut()) {
            *out = PALETTE[*shade as usize];
        }
        window
            .update_with_buffer(&buffer, WIDTH, HEIGHT)
            .expect("could not present the frame");
    }
}

/// One instruction, plus the clock it consumed. Returns true when that pushed
/// the PPU into VBlank, meaning a frame is ready to present.
fn step_once<W: Write>(cpu: &mut Cpu, bus: &mut Bus, trace: &mut Option<W>) -> bool {
    // gameboy-doctor wants one line per executed opcode, and neither an
    // interrupt dispatch nor a halted step executes one.
    if cpu.will_execute(bus) {
        if let Some(out) = trace.as_mut() {
            cpu.write_trace(bus, out).expect("failed to write the trace");
        }
    }
    let cycles = cpu.step(bus);
    bus.tick(cycles)
}
