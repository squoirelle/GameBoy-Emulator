const FLAG_Z: u8 = 0x80;
const FLAG_N: u8 = 0x40;
const FLAG_H: u8 = 0x20;
const FLAG_C: u8 = 0x10;

#[derive(Debug, Clone, Copy)]
pub struct Registers {
    pub a: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    f: u8,
    pub h: u8,
    pub l: u8,
    pub pc: u16,
    pub sp: u16,
}

impl Registers {
    pub fn post_boot() -> Registers {
        Registers {
            a: 0x01,
            f: 0xB0,
            b: 0x00,
            c: 0x13,
            d: 0x00,
            e: 0xD8,
            h: 0x01,
            l: 0x4D,
            sp: 0xFFFE,
            pc: 0x0100,
        }
    }

    pub fn bc(&self) -> u16 {
        self.c as u16 | ((self.b as u16) << 8)
    }

    pub fn set_bc(&mut self, value: u16) {
        self.b = (value >> 8) as u8;
        self.c = value as u8
    }

    pub fn de(&self) -> u16 {
        self.e as u16 | ((self.d as u16) << 8)
    }

    pub fn set_de(&mut self, value: u16) {
        self.d = (value >> 8) as u8;
        self.e = value as u8
    }

    pub fn af(&self) -> u16 {
        self.f as u16 | ((self.a as u16) << 8)
    }

    pub fn set_af(&mut self, value: u16) {
        self.a = (value >> 8) as u8;
        self.f = (value as u8) & 0xF0;
    }

    pub fn hl(&self) -> u16 {
        self.l as u16 | ((self.h as u16) << 8)
    }

    pub fn set_hl(&mut self, value: u16) {
        self.h = (value >> 8) as u8;
        self.l = value as u8
    }

    pub fn f(&self) -> u8 {
        self.f
    }

    pub fn set_flags(&mut self, z: bool, n: bool, h: bool, c: bool) {
        self.set_flag_z(z);
        self.set_flag_n(n);
        self.set_flag_h(h);
        self.set_flag_c(c);
    }

    pub fn flag_z(&self) -> bool {
        self.f & FLAG_Z != 0
    }

    pub fn flag_n(&self) -> bool {
        self.f & FLAG_N != 0
    }

    pub fn flag_h(&self) -> bool {
        self.f & FLAG_H != 0
    }

    pub fn flag_c(&self) -> bool {
        self.f & FLAG_C != 0
    }

    pub fn set_flag_z(&mut self, on: bool) {
        self.set_flag(FLAG_Z, on)
    }

    pub fn set_flag_n(&mut self, on: bool) {
        self.set_flag(FLAG_N, on)
    }

    pub fn set_flag_h(&mut self, on: bool) {
        self.set_flag(FLAG_H, on)
    }

    pub fn set_flag_c(&mut self, on: bool) {
        self.set_flag(FLAG_C, on)
    }

    /// Sets or clears one flag bit, leaving the other three alone. Private, so
    /// the only bits anything can touch are the four that physically exist.
    fn set_flag(&mut self, mask: u8, on: bool) {
        if on {
            self.f |= mask;
        } else {
            self.f &= !mask;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_boot_matches_the_state_the_boot_rom_leaves_behind() {
        let regs = Registers::post_boot();
        assert_eq!(regs.af(), 0x01B0);
        assert_eq!(regs.bc(), 0x0013);
        assert_eq!(regs.de(), 0x00D8);
        assert_eq!(regs.hl(), 0x014D);
        assert_eq!(regs.sp, 0xFFFE);
        assert_eq!(regs.pc, 0x0100);
    }

    #[test]
    fn pairs_put_the_high_byte_in_the_first_register() {
        let mut regs = Registers::post_boot();

        regs.set_bc(0x1234);
        assert_eq!((regs.b, regs.c), (0x12, 0x34));

        regs.set_de(0x1234);
        assert_eq!((regs.d, regs.e), (0x12, 0x34));

        regs.set_hl(0x1234);
        assert_eq!((regs.h, regs.l), (0x12, 0x34));

        regs.set_af(0x1234);
        assert_eq!(regs.a, 0x12);
    }

    #[test]
    fn pairs_round_trip() {
        let mut regs = Registers::post_boot();

        for value in [0x0000u16, 0x0001, 0x1234, 0xABCD, 0xFFFF] {
            regs.set_bc(value);
            assert_eq!(regs.bc(), value, "BC failed on {value:#06X}");

            regs.set_de(value);
            assert_eq!(regs.de(), value, "DE failed on {value:#06X}");

            regs.set_hl(value);
            assert_eq!(regs.hl(), value, "HL failed on {value:#06X}");
        }
    }

    #[test]
    fn writing_a_pair_leaves_the_others_alone() {
        let mut regs = Registers::post_boot();
        regs.set_hl(0xFFFF);
        assert_eq!(regs.bc(), 0x0013);
        assert_eq!(regs.de(), 0x00D8);
        assert_eq!(regs.af(), 0x01B0);
    }

    #[test]
    fn af_drops_the_flag_registers_low_nibble() {
        let mut regs = Registers::post_boot();

        // F has only four flip-flops, wired to bits 7-4. Bits 3-0 do not exist,
        // so they never store anything and always read back as zero.
        regs.set_af(0x123F);
        assert_eq!(regs.af(), 0x1230);
        assert_eq!(regs.f, 0x30);

        // A itself is a full 8-bit register -- nothing is masked there.
        regs.set_af(0xFFFF);
        assert_eq!(regs.a, 0xFF);
        assert_eq!(regs.af(), 0xFFF0);
    }

    #[test]
    fn each_flag_owns_exactly_one_bit() {
        let mut regs = Registers::post_boot();

        regs.set_af(0x0000); // clear every flag
        for (name, set, get) in [
            (
                "Z",
                Registers::set_flag_z as fn(&mut Registers, bool),
                Registers::flag_z as fn(&Registers) -> bool,
            ),
            ("N", Registers::set_flag_n, Registers::flag_n),
            ("H", Registers::set_flag_h, Registers::flag_h),
            ("C", Registers::set_flag_c, Registers::flag_c),
        ] {
            set(&mut regs, true);
            assert!(get(&regs), "{name} did not set");

            set(&mut regs, false);
            assert!(!get(&regs), "{name} did not clear");
            assert_eq!(regs.f(), 0x00, "{name} disturbed another flag");
        }
    }

    #[test]
    fn setting_flags_never_touches_the_low_nibble() {
        let mut regs = Registers::post_boot();

        regs.set_flag_z(true);
        regs.set_flag_n(true);
        regs.set_flag_h(true);
        regs.set_flag_c(true);
        assert_eq!(regs.f(), 0xF0);

        regs.set_flag_z(false);
        regs.set_flag_h(false);
        assert_eq!(regs.f(), 0x50); // N and C only
    }
}
