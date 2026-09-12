# Game Boy emulator

A DMG (original, monochrome Game Boy) emulator written in Rust, as an exercise
in learning the language. Roughly 2,800 lines across eight modules, with
`minifb` for the window as the only dependency — the CPU, PPU, timer, joypad,
bus and cartridge mappers are all written from scratch against the hardware
documentation.

It boots and plays commercial games, and passes Blargg's CPU test suite.

## Building and running

```
cargo run --release -- path/to/rom.gb
```

The debug build is around ten times slower — still comfortably above full
speed on a modern machine, but release is what you want for turbo and for
running the test ROMs.

With no argument it looks for `Tetris.GB` in the working directory.

The window opens at 4× (640×576) and can be resized; the picture keeps the
Game Boy's 10:9 aspect ratio and letterboxes the rest.

## Controls

| Key | Button |
| --- | --- |
| Arrow keys | D-pad |
| <kbd>Z</kbd> | A |
| <kbd>X</kbd> | B |
| <kbd>Enter</kbd> | Start |
| <kbd>Backspace</kbd> | Select |
| <kbd>Tab</kbd> (hold) | Turbo — drops the frame limiter |
| <kbd>Esc</kbd> | Quit |

## Saves

A cartridge with battery-backed RAM reads a `.sav` beside the ROM on load and
writes it back on a clean exit. A missing file just means a new game, and a
cartridge without a battery never writes one. Being killed or panicking loses
the session, the same way pulling a real cartridge's battery would.

## Debugging switches

All are environment variables, all off by default.

| Variable | Effect |
| --- | --- |
| `GB_TRACE=file` | Write one line per executed instruction to `file`, in gameboy-doctor's format |
| `GB_STEPS=n` | Run exactly `n` instructions with no window, then exit |
| `GB_ASCII=1` | After a `GB_STEPS` run, print the last frame as text plus a line of PPU state |
| `GB_DOCTOR=1` | Pin `LY` to `0x90`, which is what gameboy-doctor's reference logs were captured with |

`GB_STEPS` and `GB_ASCII` together are how the test ROMs below are run: they
report on screen, and the ASCII dump reads their result without a window in
the way.

```
GB_STEPS=250000000 GB_ASCII=1 cargo run --release -- gb-test-roms/cpu_instrs/cpu_instrs.gb
```

## Accuracy

47 unit tests cover the register file, timer, PPU schedule, joypad matrix and
bus. `cargo test` runs them.

Against Blargg's test ROMs:

| ROM | Result |
| --- | --- |
| `cpu_instrs` (all 11) | Passed |
| `instr_timing` | Passed |
| `halt_bug` | Passed |
| `mem_timing`, `mem_timing-2` | Failed, by construction |
| `oam_bug`, `interrupt_time` | Failed, by construction |

The last four measure behaviour *within* an instruction. Execution here is
instruction-stepped — an instruction runs to completion and then hands its
whole cycle cost to the PPU and timer at once — so sub-instruction memory
timing is not something this design can express. Fixing that means rewriting
the CPU as a cycle-stepped machine, which is a deliberate non-goal for now.

The PPU renders a whole scanline at a time rather than emulating the pixel
FIFO, so mid-scanline register writes land at the wrong place. Background,
window and sprites (8×8 and 8×16, with flips and priority) are all drawn.

Cartridge mappers: no-MBC, MBC1, MBC3 and MBC5.

## Not implemented

- **Sound.** There is no APU at all; the audio registers read and write as
  plain memory.
- **Game Boy Color.** DMG only — the colour-only registers deliberately read
  back `0xFF` so that ROMs detect the absence of the hardware.
- **MBC2**, and **MBC3's real-time clock** (the header is parsed and reported,
  but the clock does not tick).
- **The serial link.** Bytes written to `SB` are collected so that test ROMs
  can report, but there is nothing on the other end of the cable.

## Layout

| File | |
| --- | --- |
| `src/cpu.rs` | Instruction decode and execution, interrupt dispatch, the halt bug |
| `src/registers.rs` | The register file, the 16-bit pairs, and the flag bits |
| `src/bus.rs` | Address decoding, I/O registers, DMA, the serial port |
| `src/cartridge.rs` | Header parsing and the MBC banking logic |
| `src/ppu.rs` | The LCD schedule, STAT/LY, and the scanline renderer |
| `src/timer.rs` | DIV, TIMA, TMA and TAC |
| `src/joypad.rs` | The button matrix behind `P1` |
| `src/main.rs` | Argument handling, the window, the frame loop, saves |

## References

- [Pan Docs](https://gbdev.io/pandocs/) — the hardware reference this was written against
- [The gbdev opcode table](https://gbdev.io/gb-opcodes/optables/) — instruction encodings, flags and cycle counts
- [Blargg's test ROMs](https://github.com/retrio/gb-test-roms)
- [gameboy-doctor](https://github.com/robert/gameboy-doctor) — diffs an instruction trace against a known-good log
- [dmg-acid2](https://github.com/mattcurrie/dmg-acid2) — a PPU rendering test

## Licence

Dual licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT licence ([LICENSE-MIT](LICENSE-MIT))

at your option.

No ROMs are included in this repository, and `.gb`/`.gbc` files are
gitignored.
