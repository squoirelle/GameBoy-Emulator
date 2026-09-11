//! Cartridge loading, header parsing, and memory bank controllers.
//!
//! The cartridge answers two windows of the CPU address space:
//!   0x0000-0x7FFF  ROM  (bank 0 + a switchable bank)
//!   0xA000-0xBFFF  RAM  (battery-backed on most carts)
//!
//! Writes into the ROM window are not memory writes: they are commands to the
//! bank controller chip on the cartridge board.

use std::fmt;
use std::fs;
use std::path::Path;

// Header field offsets, all within the first 0x150 bytes of every cartridge.
const TITLE_START: usize = 0x0134;
const TITLE_END_OLD: usize = 0x0143; // inclusive; on CGB carts this byte is the CGB flag
const CGB_FLAG: usize = 0x0143;
const CART_TYPE: usize = 0x0147;
const ROM_SIZE_CODE: usize = 0x0148;
const RAM_SIZE_CODE: usize = 0x0149;
const HEADER_CHECKSUM: usize = 0x014D;

const ROM_BANK_SIZE: usize = 0x4000; // 16 KiB
const RAM_BANK_SIZE: usize = 0x2000; // 8 KiB
const HEADER_END: usize = 0x0150;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum CartError {
    Io(std::io::Error),
    TooShort(usize),
    BadChecksum { expected: u8, found: u8 },
    UnknownCartType(u8),
    UnknownRomSize(u8),
    UnknownRamSize(u8),
}

impl fmt::Display for CartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CartError::Io(e) => write!(f, "could not read ROM file: {e}"),
            CartError::TooShort(len) => {
                write!(
                    f,
                    "file is {len} bytes, too short to contain a cartridge header"
                )
            }
            CartError::BadChecksum { expected, found } => write!(
                f,
                "header checksum mismatch: header says {expected:#04X}, computed {found:#04X}"
            ),
            CartError::UnknownCartType(c) => write!(f, "unsupported cartridge type {c:#04X}"),
            CartError::UnknownRomSize(c) => write!(f, "unknown ROM size code {c:#04X}"),
            CartError::UnknownRamSize(c) => write!(f, "unknown RAM size code {c:#04X}"),
        }
    }
}

impl std::error::Error for CartError {}

impl From<std::io::Error> for CartError {
    fn from(e: std::io::Error) -> Self {
        CartError::Io(e)
    }
}

// ---------------------------------------------------------------------------
// Header: immutable facts, decoded once at load
// ---------------------------------------------------------------------------

/// Which bank controller chip is on the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MbcKind {
    None,
    Mbc1,
    Mbc3,
    Mbc5,
}

impl fmt::Display for MbcKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            MbcKind::None => "no MBC",
            MbcKind::Mbc1 => "MBC1",
            MbcKind::Mbc3 => "MBC3",
            MbcKind::Mbc5 => "MBC5",
        };
        f.write_str(name)
    }
}

#[derive(Debug, Clone)]
pub struct Header {
    pub title: String,
    pub mbc: MbcKind,
    pub rom_banks: usize,
    pub ram_banks: usize,
    /// The cartridge type byte claims RAM exists. Carts occasionally disagree
    /// with the RAM size byte; `ram_banks` is what we actually allocate.
    pub has_ram: bool,
    pub has_battery: bool,
    pub has_rtc: bool,
    pub cgb: bool,
}

impl Header {
    pub fn rom_size(&self) -> usize {
        self.rom_banks * ROM_BANK_SIZE
    }

    pub fn ram_size(&self) -> usize {
        self.ram_banks * RAM_BANK_SIZE
    }

    fn parse(rom: &[u8]) -> Result<Header, CartError> {
        if rom.len() < HEADER_END {
            return Err(CartError::TooShort(rom.len()));
        }

        let expected = rom[HEADER_CHECKSUM];
        let found = header_checksum(rom);
        if expected != found {
            return Err(CartError::BadChecksum { expected, found });
        }

        // On CGB-era carts 0x0143 is the CGB flag and the title is one byte shorter.
        let cgb = matches!(rom[CGB_FLAG], 0x80 | 0xC0);
        let title_end = if cgb {
            TITLE_END_OLD - 1
        } else {
            TITLE_END_OLD
        };
        let title = String::from_utf8_lossy(&rom[TITLE_START..=title_end])
            .trim_end_matches(|c: char| c == '\0' || c == ' ')
            .to_string();

        let ct = CartType::decode(rom[CART_TYPE])?;

        Ok(Header {
            title,
            mbc: ct.kind,
            rom_banks: decode_rom_banks(rom[ROM_SIZE_CODE])?,
            ram_banks: decode_ram_banks(rom[RAM_SIZE_CODE])?,
            has_ram: ct.ram,
            has_battery: ct.battery,
            has_rtc: ct.rtc,
            cgb,
        })
    }
}

impl fmt::Display for Header {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Title      : {}", self.title)?;
        writeln!(f, "Controller : {}", self.mbc)?;
        writeln!(
            f,
            "ROM        : {} KiB ({} banks)",
            self.rom_size() / 1024,
            self.rom_banks
        )?;
        if self.ram_banks == 0 {
            // The two header bytes disagree often enough to be worth saying so
            // out loud, since it decides whether a .sav is ever written.
            if self.has_ram {
                writeln!(f, "RAM        : none (the cartridge type claims some)")?;
            } else {
                writeln!(f, "RAM        : none")?;
            }
        } else {
            writeln!(
                f,
                "RAM        : {} KiB ({} banks)",
                self.ram_size() / 1024,
                self.ram_banks
            )?;
        }
        writeln!(
            f,
            "Battery    : {}",
            if self.has_battery { "yes" } else { "no" }
        )?;
        // Only MBC3 carries a clock, so saying "no" on every other cartridge
        // would be noise.
        if self.has_rtc {
            writeln!(f, "RTC        : yes")?;
        }
        write!(f, "Mode       : {}", if self.cgb { "CGB" } else { "DMG" })
    }
}

/// Everything the cartridge-type byte encodes. It is four facts, not one.
struct CartType {
    kind: MbcKind,
    ram: bool,
    battery: bool,
    rtc: bool,
}

impl CartType {
    fn decode(code: u8) -> Result<CartType, CartError> {
        let (kind, ram, battery, rtc) = match code {
            0x00 => (MbcKind::None, false, false, false),
            0x08 => (MbcKind::None, true, false, false),
            0x09 => (MbcKind::None, true, true, false),

            0x01 => (MbcKind::Mbc1, false, false, false),
            0x02 => (MbcKind::Mbc1, true, false, false),
            0x03 => (MbcKind::Mbc1, true, true, false),

            0x0F => (MbcKind::Mbc3, false, true, true),
            0x10 => (MbcKind::Mbc3, true, true, true),
            0x11 => (MbcKind::Mbc3, false, false, false),
            0x12 => (MbcKind::Mbc3, true, false, false),
            0x13 => (MbcKind::Mbc3, true, true, false),

            // 0x1C-0x1E additionally have a rumble motor, which we ignore.
            0x19 => (MbcKind::Mbc5, false, false, false),
            0x1A => (MbcKind::Mbc5, true, false, false),
            0x1B => (MbcKind::Mbc5, true, true, false),
            0x1C => (MbcKind::Mbc5, false, false, false),
            0x1D => (MbcKind::Mbc5, true, false, false),
            0x1E => (MbcKind::Mbc5, true, true, false),

            // MBC2, MMM01, HuC1 and friends exist but are not supported here.
            other => return Err(CartError::UnknownCartType(other)),
        };
        Ok(CartType {
            kind,
            ram,
            battery,
            rtc,
        })
    }
}

fn decode_rom_banks(code: u8) -> Result<usize, CartError> {
    match code {
        0x00..=0x08 => Ok(2usize << code), // 32 KiB doubling up to 8 MiB
        other => Err(CartError::UnknownRomSize(other)),
    }
}

fn decode_ram_banks(code: u8) -> Result<usize, CartError> {
    match code {
        0x00 => Ok(0),
        0x02 => Ok(1),  // 8 KiB
        0x03 => Ok(4),  // 32 KiB
        0x04 => Ok(16), // 128 KiB
        0x05 => Ok(8),  // 64 KiB -- yes, out of order
        other => Err(CartError::UnknownRamSize(other)),
    }
}

/// Sum of 0x0134..=0x014C, each byte subtracted along with 1. Wraps at 8 bits.
fn header_checksum(rom: &[u8]) -> u8 {
    let mut sum: u8 = 0;
    for &byte in &rom[TITLE_START..=0x014C] {
        sum = sum.wrapping_sub(byte).wrapping_sub(1);
    }
    sum
}

// ---------------------------------------------------------------------------
// MBC: the live register state of the controller chip
// ---------------------------------------------------------------------------

/// Runtime banking state. `MbcKind` says which chip is fitted; this says what
/// its registers currently hold.
#[derive(Debug, Clone)]
enum Mbc {
    None,
    /// MBC1 has a 5-bit register and a 2-bit register. The 2-bit one is either
    /// the top ROM bank bits or the RAM bank, depending on `mode`.
    Mbc1 {
        bank1: u8,
        bank2: u8,
        mode: bool,
        ram_enabled: bool,
    },
    Mbc3 {
        rom_bank: u8,
        ram_bank: u8,
        ram_enabled: bool,
    },
    Mbc5 {
        rom_bank: u16,
        ram_bank: u8,
        ram_enabled: bool,
    },
}

impl Mbc {
    fn new(kind: MbcKind) -> Mbc {
        match kind {
            MbcKind::None => Mbc::None,
            MbcKind::Mbc1 => Mbc::Mbc1 {
                bank1: 1,
                bank2: 0,
                mode: false,
                ram_enabled: false,
            },
            MbcKind::Mbc3 => Mbc::Mbc3 {
                rom_bank: 1,
                ram_bank: 0,
                ram_enabled: false,
            },
            MbcKind::Mbc5 => Mbc::Mbc5 {
                rom_bank: 1,
                ram_bank: 0,
                ram_enabled: false,
            },
        }
    }

    /// Bank mapped at 0x0000-0x3FFF. Only MBC1 in advanced mode moves this.
    fn low_rom_bank(&self) -> usize {
        match self {
            Mbc::Mbc1 {
                bank2, mode: true, ..
            } => (*bank2 as usize) << 5,
            _ => 0,
        }
    }

    /// Bank mapped at 0x4000-0x7FFF.
    fn high_rom_bank(&self) -> usize {
        match self {
            Mbc::None => 1,
            Mbc::Mbc1 { bank1, bank2, .. } => ((*bank2 as usize) << 5) | (*bank1 as usize),
            Mbc::Mbc3 { rom_bank, .. } => *rom_bank as usize,
            Mbc::Mbc5 { rom_bank, .. } => *rom_bank as usize,
        }
    }

    fn ram_bank(&self) -> usize {
        match self {
            Mbc::None => 0,
            Mbc::Mbc1 { bank2, mode, .. } => {
                if *mode {
                    *bank2 as usize
                } else {
                    0
                }
            }
            Mbc::Mbc3 { ram_bank, .. } => *ram_bank as usize,
            Mbc::Mbc5 { ram_bank, .. } => *ram_bank as usize,
        }
    }

    fn ram_enabled(&self) -> bool {
        match self {
            Mbc::None => true,
            Mbc::Mbc1 { ram_enabled, .. }
            | Mbc::Mbc3 { ram_enabled, .. }
            | Mbc::Mbc5 { ram_enabled, .. } => *ram_enabled,
        }
    }

    /// Handle a write into the 0x0000-0x7FFF command range.
    fn control_write(&mut self, addr: u16, value: u8) {
        match self {
            Mbc::None => {}

            Mbc::Mbc1 {
                bank1,
                bank2,
                mode,
                ram_enabled,
            } => match addr {
                0x0000..=0x1FFF => *ram_enabled = value & 0x0F == 0x0A,
                // A 5-bit register that can never hold 0: writing 0 selects 1.
                0x2000..=0x3FFF => *bank1 = if value & 0x1F == 0 { 1 } else { value & 0x1F },
                0x4000..=0x5FFF => *bank2 = value & 0x03,
                0x6000..=0x7FFF => *mode = value & 0x01 == 1,
                _ => unreachable!(),
            },

            Mbc::Mbc3 {
                rom_bank,
                ram_bank,
                ram_enabled,
            } => match addr {
                0x0000..=0x1FFF => *ram_enabled = value & 0x0F == 0x0A,
                0x2000..=0x3FFF => *rom_bank = if value & 0x7F == 0 { 1 } else { value & 0x7F },
                // 0x08-0x0C select the real-time clock registers instead of RAM.
                // Not implemented: those reads will fall through to cart RAM.
                0x4000..=0x5FFF => *ram_bank = value & 0x0F,
                0x6000..=0x7FFF => {} // RTC latch
                _ => unreachable!(),
            },

            Mbc::Mbc5 {
                rom_bank,
                ram_bank,
                ram_enabled,
            } => match addr {
                0x0000..=0x1FFF => *ram_enabled = value == 0x0A,
                // MBC5 splits the 9-bit bank number across two ranges, and
                // unlike MBC1 it can genuinely select bank 0.
                0x2000..=0x2FFF => *rom_bank = (*rom_bank & 0x100) | value as u16,
                0x3000..=0x3FFF => *rom_bank = (*rom_bank & 0x0FF) | ((value as u16 & 1) << 8),
                0x4000..=0x5FFF => *ram_bank = value & 0x0F,
                0x6000..=0x7FFF => {}
                _ => unreachable!(),
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Cartridge
// ---------------------------------------------------------------------------

pub struct Cartridge {
    pub header: Header,
    rom: Vec<u8>,
    ram: Vec<u8>,
    mbc: Mbc,
}

impl Cartridge {
    pub fn load(path: &Path) -> Result<Cartridge, CartError> {
        Cartridge::from_bytes(fs::read(path)?)
    }

    pub fn from_bytes(rom: Vec<u8>) -> Result<Cartridge, CartError> {
        let header = Header::parse(&rom)?;
        let mbc = Mbc::new(header.mbc);
        let ram = vec![0u8; header.ram_size()];
        Ok(Cartridge {
            header,
            rom,
            ram,
            mbc,
        })
    }

    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x3FFF => self.rom_byte(self.mbc.low_rom_bank(), addr as usize),
            0x4000..=0x7FFF => {
                self.rom_byte(self.mbc.high_rom_bank(), addr as usize - ROM_BANK_SIZE)
            }
            0xA000..=0xBFFF => match self.ram_index(addr) {
                Some(i) => self.ram[i],
                None => 0xFF, // disabled or absent RAM reads as open bus
            },
            _ => 0xFF,
        }
    }

    pub fn write(&mut self, addr: u16, value: u8) {
        match addr {
            0x0000..=0x7FFF => self.mbc.control_write(addr, value),
            0xA000..=0xBFFF => {
                if let Some(i) = self.ram_index(addr) {
                    self.ram[i] = value;
                }
            }
            _ => {}
        }
    }

    /// Cartridge RAM, for writing a .sav file.
    pub fn ram(&self) -> &[u8] {
        &self.ram
    }

    /// Restore cartridge RAM from a .sav file. Wrong-sized data is ignored.
    pub fn load_ram(&mut self, data: &[u8]) {
        if data.len() == self.ram.len() {
            self.ram.copy_from_slice(data);
        }
    }

    /// Bank numbers wrap around the cartridge's real size, so a game selecting
    /// bank 40 on a 16-bank cart reads bank 8 rather than panicking.
    fn rom_byte(&self, bank: usize, offset: usize) -> u8 {
        let bank = bank & (self.header.rom_banks - 1);
        self.rom[bank * ROM_BANK_SIZE + offset]
    }

    fn ram_index(&self, addr: u16) -> Option<usize> {
        if self.ram.is_empty() || !self.mbc.ram_enabled() {
            return None;
        }
        let bank = self.mbc.ram_bank() & (self.header.ram_banks - 1);
        Some(bank * RAM_BANK_SIZE + (addr as usize - 0xA000))
    }
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// Every cartridge carries this identical 48-byte bitmap at 0x0104-0x0133: it
/// is the "Nintendo" logo the boot ROM scrolls down the screen, and the boot
/// ROM refuses to start a cartridge whose copy does not match byte for byte.
#[cfg(test)]
const NINTENDO_LOGO: [u8; 48] = [
    0xCE, 0xED, 0x66, 0x66, 0xCC, 0x0D, 0x00, 0x0B, 0x03, 0x73, 0x00, 0x83, 0x00, 0x0C, 0x00, 0x0D,
    0x00, 0x08, 0x11, 0x1F, 0x88, 0x89, 0x00, 0x0E, 0xDC, 0xCC, 0x6E, 0xE6, 0xDD, 0xDD, 0xD9, 0x99,
    0xBB, 0xBB, 0x67, 0x63, 0x6E, 0x0E, 0xEC, 0xCC, 0xDD, 0xDC, 0x99, 0x9F, 0xBB, 0xB9, 0x33, 0x3E,
];

/// Builds a syntactically valid cartridge image with a correct checksum, so
/// tests can make cartridges without a ROM file on disk. Shared with the other
/// modules' test suites, hence `pub(crate)`.
#[cfg(test)]
pub(crate) fn fake_rom(title: &str, cart_type: u8, rom_code: u8, ram_code: u8) -> Vec<u8> {
    let banks = decode_rom_banks(rom_code).unwrap();
    let mut rom = vec![0u8; banks * ROM_BANK_SIZE];

    rom[0x0104..0x0104 + NINTENDO_LOGO.len()].copy_from_slice(&NINTENDO_LOGO);

    for (i, b) in title.bytes().take(16).enumerate() {
        rom[TITLE_START + i] = b;
    }
    rom[CART_TYPE] = cart_type;
    rom[ROM_SIZE_CODE] = rom_code;
    rom[RAM_SIZE_CODE] = ram_code;
    rom[HEADER_CHECKSUM] = header_checksum(&rom);

    // Stamp each bank with its own number so bank switching is observable.
    for bank in 0..banks {
        rom[bank * ROM_BANK_SIZE + 0x0010] = bank as u8;
    }
    rom
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_plain_32k_cartridge() {
        let cart = Cartridge::from_bytes(fake_rom("TETRIS", 0x00, 0x00, 0x00)).unwrap();
        assert_eq!(cart.header.title, "TETRIS");
        assert_eq!(cart.header.mbc, MbcKind::None);
        assert_eq!(cart.header.rom_size(), 32 * 1024);
        assert_eq!(cart.header.ram_banks, 0);
        assert!(!cart.header.has_battery);
    }

    #[test]
    fn rejects_a_corrupt_header() {
        let mut rom = fake_rom("TETRIS", 0x00, 0x00, 0x00);
        rom[HEADER_CHECKSUM] ^= 0xFF;
        assert!(matches!(
            Cartridge::from_bytes(rom),
            Err(CartError::BadChecksum { .. })
        ));
    }

    #[test]
    fn mbc1_switches_rom_banks() {
        // 0x03 = MBC1 + RAM + BATTERY, 0x05 = 1 MiB (64 banks), 0x03 = 32 KiB RAM
        let mut cart = Cartridge::from_bytes(fake_rom("BANKS", 0x03, 0x05, 0x03)).unwrap();
        assert!(cart.header.has_battery);

        // Bank 1 is mapped at 0x4000 out of reset.
        assert_eq!(cart.read(0x4010), 1);

        cart.write(0x2000, 5);
        assert_eq!(cart.read(0x4010), 5);

        // Writing 0 to the 5-bit register selects bank 1, never bank 0.
        cart.write(0x2000, 0);
        assert_eq!(cart.read(0x4010), 1);

        // The 2-bit register supplies bits 5-6: bank2=1, bank1=2 -> bank 34.
        cart.write(0x2000, 2);
        cart.write(0x4000, 1);
        assert_eq!(cart.read(0x4010), 34);

        // Bank 0 stays at 0x0000 until advanced banking mode is switched on.
        assert_eq!(cart.read(0x0010), 0);
        cart.write(0x6000, 1);
        assert_eq!(cart.read(0x0010), 32);
    }

    #[test]
    fn cart_ram_needs_enabling() {
        let mut cart = Cartridge::from_bytes(fake_rom("SAVE", 0x03, 0x00, 0x02)).unwrap();

        // Disabled out of reset: writes are dropped, reads are open bus.
        cart.write(0xA000, 0x42);
        assert_eq!(cart.read(0xA000), 0xFF);

        cart.write(0x0000, 0x0A);
        cart.write(0xA000, 0x42);
        assert_eq!(cart.read(0xA000), 0x42);
        assert_eq!(cart.ram()[0], 0x42);
    }
}
