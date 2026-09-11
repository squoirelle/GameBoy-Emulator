use crate::cartridge::Cartridge;
use crate::joypad::{Button, Joypad};
use crate::ppu::Ppu;
use crate::timer::Timer;
use std::io::Write;

// I/O registers that mean something to us today. The rest of 0xFF00-0xFF7F is
// still plain storage until the timer and PPU arrive.
const SB: u16 = 0xFF01; // serial transfer data
const SC: u16 = 0xFF02; // serial transfer control

const IO_BASE: u16 = 0xFF00;

pub struct Bus {
    cartridge: Cartridge,
    wram: [u8; 0x2000],
    io: [u8; 0x80],
    hram: [u8; 0x7F],
    ie: u8,
    iflag: u8,
    /// Everything the cartridge has sent over the serial port. Test ROMs report
    /// their results here, so this is the harness for step 3.
    pub serial: String,
    /// gameboy-doctor needs LY pinned to 0x90 or every trace diverges against
    /// its reference logs. Real games poll for specific scanlines and hang on
    /// a constant, so the two modes are mutually exclusive.
    ly_fixed: bool,
    timer: Timer,
    joypad: Joypad,
    ppu: Ppu,
}

impl Bus {
    pub fn new(cartridge: Cartridge) -> Bus {
        Bus {
            cartridge,
            wram: [0; 0x2000],
            io: [0; 0x80],
            hram: [0; 0x7F],
            iflag: 0,
            ie: 0,
            serial: String::new(),
            ly_fixed: false,
            timer: Timer::new(),
            joypad: Joypad::new(),
            ppu: Ppu::new()
        }
    }

    /// Pins LY to 0x90 for gameboy-doctor runs. Leave it off for real games,
    /// which poll for a specific scanline and would spin forever on a constant.
    pub fn set_ly_fixed(&mut self, fixed: bool) {
        self.ly_fixed = fixed;
    }

    pub fn framebuffer(&self) -> &[u8] {
        &self.ppu.framebuffer
    }

    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x7FFF | 0xA000..=0xBFFF => self.cartridge.read(addr),
            0x8000..=0x9FFF => self.ppu.vram[(addr - 0x8000) as usize],
            0xC000..=0xFDFF => self.wram[addr as usize & 0x1FFF],
            0xFE00..=0xFE9F => self.ppu.oam[(addr - 0xFE00) as usize],
            0xFF00 => self.joypad.read(),
            // Colour-only registers. They do not exist on a DMG and read as
            // 0xFF, which is how a ROM tells the two machines apart. Reading
            // 0x00 instead tells it the hardware is there: Blargg's cpu_instrs
            // writes KEY1 and executes STOP to switch to double speed, and
            // lands on an instruction no DMG game ever runs.
            0xFF4D | 0xFF4F | 0xFF51..=0xFF55 | 0xFF68..=0xFF6B | 0xFF70 => 0xFF,
            0xFF0F => self.iflag | 0xE0,
            0xFF04 => self.timer.div(),
            0xFF05 => self.timer.tima,
            0xFF06 => self.timer.tma,
            0xFF07 => self.timer.tac | 0xF8,
            0xFF40 => self.ppu.lcdc,
            0xFF41 => self.ppu.stat(),
            0xFF42 => self.ppu.scy,
            0xFF43 => self.ppu.scx,
            0xFF44 => if self.ly_fixed { 0x90 } else { self.ppu.ly },
            0xFF45 => self.ppu.lyc,
            0xFF47 => self.ppu.bgp,
            0xFF48 => self.ppu.obp0,
            0xFF49 => self.ppu.obp1,
            0xFF4A => self.ppu.wy,
            0xFF4B => self.ppu.wx,
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
            0x8000..=0x9FFF => self.ppu.vram[(addr - 0x8000) as usize] = value,
            0xC000..=0xFDFF => self.wram[addr as usize & 0x1FFF] = value,
            0xFE00..=0xFE9F => self.ppu.oam[(addr - 0xFE00) as usize] = value,
            SC => {
                self.io[(SC - IO_BASE) as usize] = value;
                // Bit 7 starts a transfer. Other writes only pick a clock
                // source and must not emit anything.
                if value & 0x80 != 0 {
                    self.transfer_serial();
                }
            }
            0xFF04 => self.timer.reset_div(),
            0xFF05 => self.timer.tima = value,
            0xFF06 => self.timer.tma = value,
            0xFF07 => self.timer.tac = value & 0x07,
            0xFF40 => self.ppu.set_lcdc(value),
            0xFF41 => self.ppu.set_stat(value),
            0xFF42 => self.ppu.scy = value,
            0xFF43 => self.ppu.scx = value,
            0xFF44 => {}, //ignored on hardware
            0xFF45 => self.ppu.lyc = value,
            0xFF46 => {
                let base = (value as u16) << 8;
                for i in 0..0xA0u16 {
                    let byte = self.read(base + i);
                    self.ppu.oam[i as usize] = byte;
                }
            }
            0xFF47 => self.ppu.bgp = value,
            0xFF48 => self.ppu.obp0 = value,
            0xFF49 => self.ppu.obp1 = value,
            0xFF4A => self.ppu.wy = value,
            0xFF4B => self.ppu.wx = value,
            0xFF00 => self.joypad.write(value),
            0xFF0F => self.iflag = value & 0x1F,
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

    /// Advances the clocked hardware. Returns true when the PPU finished a
    /// frame, which is the frontend's cue to present one.
    pub fn tick(&mut self, cycles: u32) -> bool {
        if self.timer.tick(cycles) {
            self.request_interrupt(2);
        }

        // The PPU can raise VBlank and STAT, so it reports IF bits rather than
        // a single flag.
        let requested = self.ppu.tick(cycles);
        self.iflag |= requested;

        requested & 0x01 != 0
    }

    /// Read-only view of the cartridge, so the frontend can write a .sav.
    pub fn cartridge(&self) -> &Cartridge { &self.cartridge }

    /// Read-only view of the PPU, for diagnostics.
    pub fn ppu(&self) -> &Ppu { &self.ppu }

    /// Read-only view of sprite memory, for diagnostics.
    pub fn oam(&self) -> &[u8] { &self.ppu.oam }

    pub fn set_button(&mut self, button: Button, down: bool) {
        if self.joypad.set(button, down) {
            self.request_interrupt(4);
        }
    }

    pub fn pending_interrupts(&self) -> u8 { self.ie & self.iflag & 0x1F }
    pub fn request_interrupt(&mut self, bit: u8) { self.iflag |= 1 << bit; }
    pub fn clear_interrupt(&mut self, bit: u8) { self.iflag &= !(1 << bit); }
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
