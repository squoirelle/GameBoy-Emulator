use crate::cartridge::Cartridge;
use std::cell::Cell;
use std::io::Write;

// I/O registers that mean something to us today. The rest of 0xFF00-0xFF7F is
// still plain storage until the timer and PPU arrive.
const SB: u16 = 0xFF01; // serial transfer data
const SC: u16 = 0xFF02; // serial transfer control
const LY: u16 = 0xFF44; // current scanline

const IO_BASE: u16 = 0xFF00;

pub struct Bus {
    cartridge: Cartridge,
    vram: [u8; 0x2000],
    wram: [u8; 0x2000],
    oam: [u8; 0xA0],
    io: [u8; 0x80],
    hram: [u8; 0x7F],
    ie: u8,
    /// Everything the cartridge has sent over the serial port. Test ROMs report
    /// their results here, so this is the harness for step 3.
    pub serial: String,
    /// gameboy-doctor needs LY pinned to 0x90 or every trace diverges. Real
    /// games need it to move, so the two modes are mutually exclusive.
    ly_fixed: bool,
    /// Stand-in for the PPU's line counter until step 5. A `Cell` because it
    /// advances on read, and `read` takes `&self`.
    ly: Cell<u8>,
}

impl Bus {
    pub fn new(cartridge: Cartridge) -> Bus {
        Bus {
            cartridge,
            vram: [0; 0x2000],
            wram: [0; 0x2000],
            oam: [0; 0xA0],
            io: [0; 0x80],
            hram: [0; 0x7F],
            ie: 0,
            serial: String::new(),
            ly_fixed: false,
            ly: Cell::new(0),
        }
    }

    /// Pins LY to 0x90 for gameboy-doctor runs. Leave it off for real games,
    /// which poll for specific scanlines and hang on a constant.
    pub fn set_ly_fixed(&mut self, fixed: bool) {
        self.ly_fixed = fixed;
    }

    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x7FFF | 0xA000..=0xBFFF => self.cartridge.read(addr),
            0x8000..=0x9FFF => self.vram[(addr - 0x8000) as usize],
            0xC000..=0xFDFF => self.wram[addr as usize & 0x1FFF],
            0xFE00..=0xFE9F => self.oam[(addr - 0xFE00) as usize],
            LY => {
                if self.ly_fixed {
                    0x90
                } else {
                    // No PPU yet, so there is no elapsed time to derive a line
                    // number from. Stepping one line per read is wrong by any
                    // measure, but it makes every LY-polling loop terminate.
                    let line = (self.ly.get() + 1) % 154;
                    self.ly.set(line);
                    line
                }
            }
            0xFF00..=0xFF7F => self.io[(addr - IO_BASE) as usize],
            0xFF80..=0xFFFE => self.hram[(addr - 0xFF80) as usize],
            0xFFFF => self.ie,
            // 0xFEA0-0xFEFF is unusable and reads as open bus.
            _ => 0xFF,
        }
    }

    pub fn write(&mut self, addr: u16, value: u8) {
        match addr {
            0x0000..=0x7FFF | 0xA000..=0xBFFF => self.cartridge.write(addr, value),
            0x8000..=0x9FFF => self.vram[(addr - 0x8000) as usize] = value,
            0xC000..=0xFDFF => self.wram[addr as usize & 0x1FFF] = value,
            0xFE00..=0xFE9F => self.oam[(addr - 0xFE00) as usize] = value,
            SC => {
                self.io[(SC - IO_BASE) as usize] = value;
                // Bit 7 starts a transfer. Other writes only pick a clock
                // source and must not emit anything.
                if value & 0x80 != 0 {
                    self.transfer_serial();
                }
            }
            0xFF00..=0xFF7F => self.io[(addr - IO_BASE) as usize] = value,
            0xFF80..=0xFFFE => self.hram[(addr - 0xFF80) as usize] = value,
            0xFFFF => self.ie = value,
            // 0xFEA0-0xFEFF is unusable; hardware simply drops the write.
            _ => {}
        }
    }

    /// Sends the byte sitting in SB. Test ROMs report their results this way,
    /// so it goes to stdout as well as into the buffer.
    fn transfer_serial(&mut self) {
        let byte = self.io[(SB - IO_BASE) as usize];
        self.serial.push(byte as char);
        print!("{}", byte as char);
        let _ = std::io::stdout().flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartridge::fake_rom;

    fn test_bus() -> Bus {
        Bus::new(Cartridge::from_bytes(fake_rom("TEST", 0x00, 0x00, 0x00)).unwrap())
    }

    #[test]
    fn read_from_rom() {
        let bus = test_bus();
        assert_eq!(bus.read(0x0104), 0xCE);
    }

    #[test]
    fn echo_mirrors_wram() {
        let mut bus = test_bus();
        bus.write(0xC000, 0x12);
        assert_eq!(bus.read(0xE000), 0x12);
    }

    #[test]
    fn wram_mirrors_echo() {
        let mut bus = test_bus();
        bus.write(0xE000, 0x12);
        assert_eq!(bus.read(0xC000), 0x12);
    }

    #[test]
    fn read_from_unusable() {
        let bus = test_bus();
        assert_eq!(bus.read(0xFEA0), 0xFF);
    }

    #[test]
    fn storage_regions_round_trip() {
        let mut bus = test_bus();
        for addr in [0x8000u16, 0xC000, 0xFE00, 0xFF80, 0xFFFF] {
            bus.write(addr, 0x5A);
            assert_eq!(bus.read(addr), 0x5A, "round trip failed at {addr:#06X}");
        }
    }

    #[test]
    fn hram_and_ie_do_not_overlap() {
        let mut bus = test_bus();

        // HRAM is 0xFF80..=0xFFFE -- 127 bytes, with IE sitting just past it.
        bus.write(0xFFFE, 0x11);
        assert_eq!(bus.read(0xFFFF), 0x00);

        bus.write(0xFFFF, 0x22);
        assert_eq!(bus.read(0xFFFE), 0x11);
    }

    #[test]
    fn serial_transfer_emits_a_byte() {
        let mut bus = test_bus();
        bus.write(SB, b'P');
        bus.write(SC, 0x81);
        assert_eq!(bus.serial, "P");
    }

    #[test]
    fn serial_ignores_writes_without_the_start_bit() {
        let mut bus = test_bus();
        bus.write(SB, b'P');
        bus.write(SC, 0x01); // picks a clock source, starts nothing
        assert!(bus.serial.is_empty());
    }

    #[test]
    fn cartridge_ram_round_trips_through_the_bus() {
        // 0x03 = MBC1 + RAM + BATTERY, 0x02 = 8 KiB of cartridge RAM.
        let cart = Cartridge::from_bytes(fake_rom("SAVE", 0x03, 0x00, 0x02)).unwrap();
        let mut bus = Bus::new(cart);

        // Cartridge RAM is disabled out of reset.
        bus.write(0xA000, 0x42);
        assert_eq!(bus.read(0xA000), 0xFF);

        bus.write(0x0000, 0x0A); // MBC1 RAM enable
        bus.write(0xA000, 0x42);
        assert_eq!(bus.read(0xA000), 0x42);
    }
}
