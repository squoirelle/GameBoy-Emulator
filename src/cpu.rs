use crate::bus::Bus;
use crate::registers::Registers;
use std::io;
use std::io::Write;

#[derive(Debug, Clone, Copy)]
pub struct Cpu {
    pub regs: Registers,
    ime: bool,
    halted: bool,
    halt_bug: bool
}

impl Cpu {

    pub fn new() -> Cpu {
        Cpu{
            regs: Registers::post_boot(),
            ime: false,
            halted: false,
            halt_bug: false,
        }
    }

    pub fn will_execute(&self, bus: &Bus) -> bool {
        !self.halted && !(self.ime && bus.pending_interrupts() != 0)
    }

    pub fn step(&mut self, bus: &mut Bus) -> u32 {
        let pc = self.regs.pc;
        let pending = bus.pending_interrupts();
        if pending != 0 {
            self.halted = false;
            if self.ime {
                self.ime = false;
                let bit = pending.trailing_zeros() as u8;
                bus.clear_interrupt(bit);
                self.push16(bus, self.regs.pc);
                self.regs.pc = (0x40 + bit * 8) as u16;
                return 20;
            }
        }
        if self.halted {
            return 4;
        }
        let op = self.fetch8(bus);
        let x = op >> 6;
        let y = (op >> 3) & 7;
        let z = op & 7;
        let p = y >> 1;
        let q = y & 1;
        match (x, y, z) {
            (0, 0, 0)           => 4,                                                           // NOP
            (0, 1, 0)           => self.ld_n16_sp(bus),
            (0, 2, 0)           => panic!("STOP at {:#06X}", pc),                               // STOP
            (0, 3, 0)           => self.jr(bus),                                        // JR
            (0, _, 1) if q == 0 => self.ld_rp_n16(bus, p),                              // LD RP N16
            (0, _, 1) if q == 1 => self.add_hl_rp(p),                                   // ADD HL RP
            (0, _, 2)           => self.ld_r16_a(bus, p, q),                            // LD (BC/DE/HL+/HL-),A and LD A,(BC/DE/HL+/HL-)
            (0, _, 3) if q == 0 => self.inc_rp(p),                                      // INC RP
            (0, _, 3) if q == 1 => self.dec_rp(p),                                      // DEC RP
            (0, _, 4)           => self.inc_r8(bus, y),                                 // INC R8
            (0, _, 5)           => self.dec_r8(bus, y),                                 // DEC R8
            (0, _, 6)           => self.ld_r8_n8(bus, y),                               // LD R8 N8
            (0, 4..=7, 0)       => self.jrcc(bus, y),                                   // JR CC
            (0, 0, 7)           => self.rlca(),
            (0, 1, 7)           => self.rrca(),
            (0, 2, 7)           => self.rla(),
            (0, 3, 7)           => self.rra(),
            (0, 4, 7)           => self.daa(),
            (0, 5, 7)           => self.cpl(),
            (0, 6, 7)           => self.scf(),
            (0, 7, 7)           => self.ccf(),
            (1, 6, 6)           => self.halt(bus),                                         // HALT
            (1, _, _)           => self.ld_r8_r8(bus, y, z),                            // LD r8,r8
            (2, _, _)           => { self.alu(y, self.get_r8(R8::from_code(z), bus));
                                    if z == 6 { 8 } else { 4 } },                               // ALU[y] A, r[z]
            (3, 0, 3)           => { self.regs.pc = self.fetch16(bus); 16 },
            (3, 6, 3)           => { self.ime = false; 4 }                                      //DI
            (3, _, 6)           => { let value = self.fetch8(bus);
                               self.alu(y, value); 8}
            (3, 4 | 6, 0)       => self.ldh_n_a(bus, y),
            (3, 5 | 7, 2)       => self.ld_n16_a(bus, y),
            (3, _, 1) if q == 0 => self.pop_rp2(bus, p),
            (3, _, 1) if q == 1 && p==0 => self.ret(bus),
            (3, _, 1) if q == 1 && p==1 => {self.ime = true; self.ret(bus) },
            (3, _, 1) if q == 1 && p==2 => self.jp_hl(),
            (3, _, 1) if q == 1 && p==3 => self.ld_sp_hl(),
            (3, _, 5) if q == 0 => self.push_rp2(bus, p),
            (3, _, 5) if q == 1 && p==0 => self.call_n16(bus),
            (3, 0..=3, 2)       => self.jp_cc(bus, y),
            (3, 0..=3, 0)       => self.ret_cc(bus, y),
            (3, 0..=3, 4)       => self.call_cc(bus, y),
            (3, _, 7)           => self.rst(bus, y),
            (3, 4 | 6, 2)       => self.ld_a_c(bus, y),
            (3, 7, 3)           => { self.ime = true; 4 }
            (3, 1, 3)           => self.cb(bus),
            (3, 5, 0)           => self.add_sp_e(bus),
            (3, 7, 0)           => self.ld_hl_spe(bus),
            _                   => panic!("unimplemented opcode {:#04X} at {:#06X}", op, pc)
        }
    }

    fn pop_rp2(&mut self, bus: &mut Bus, p: u8) -> u32 {
        let v = self.pop16(bus);
        self.set_r16(R16::from_code_af(p), v);
        12
    }

    fn push_rp2(&mut self, bus: &mut Bus, p: u8) -> u32 {
        let v = self.get_r16(R16::from_code_af(p));
        self.push16(bus, v);
        16
    }

    fn push16(&mut self, bus: &mut Bus, value: u16) {
        self.regs.sp = self.regs.sp.wrapping_sub(1);
        bus.write(self.regs.sp, (value >> 8) as u8);
        self.regs.sp = self.regs.sp.wrapping_sub(1);
        bus.write(self.regs.sp, value  as u8);
    }

    fn pop16(&mut self, bus: &mut Bus) -> u16 {
        let lo = bus.read(self.regs.sp) as u16;
        self.regs.sp = self.regs.sp.wrapping_add(1);
        let hi = bus.read(self.regs.sp) as u16;
        self.regs.sp = self.regs.sp.wrapping_add(1);
        lo | (hi << 8)
    }

    fn ld_r8_r8(&mut self, bus: &mut Bus, y: u8, z: u8) -> u32 {
        let v = self.get_r8(R8::from_code(z), bus);
        self.set_r8(R8::from_code(y), bus, v);
        if y == 6 || z == 6 { 8 } else { 4 }
    }

    fn fetch8(&mut self, bus: &mut Bus) -> u8 {
        let value = bus.read(self.regs.pc);
        if self.halt_bug {
            self.halt_bug = false;
        } else {
            self.regs.pc = self.regs.pc.wrapping_add(1);
        }
        value
    }

    fn fetch16(&mut self, bus: &mut Bus) -> u16 {
        let lo = self.fetch8(bus) as u16;
        let hi = (self.fetch8(bus) as u16) << 8;
        lo | hi
    }

    fn halt(&mut self, bus: &mut Bus) -> u32 {
        if !self.ime && bus.pending_interrupts() != 0 {
            self.halt_bug = true;
        } else {
            self.halted = true;
        }
        4
    }

    fn get_r8(&self, r: R8, bus: &Bus) -> u8 {
        match r {
            R8::B => self.regs.b,
            R8::C => self.regs.c,
            R8::D => self.regs.d,
            R8::E => self.regs.e,
            R8::H => self.regs.h,
            R8::L => self.regs.l,
            R8::HlMem => bus.read(self.regs.hl()),
            R8::A => self.regs.a,
        }
    }

    fn set_r8(&mut self, r: R8, bus: &mut Bus, value: u8) {
        match r {
            R8::B => self.regs.b = value,
            R8::C => self.regs.c = value,
            R8::D => self.regs.d = value,
            R8::E => self.regs.e = value,
            R8::H => self.regs.h = value,
            R8::L => self.regs.l = value,
            R8::HlMem => bus.write(self.regs.hl(), value),
            R8::A => self.regs.a = value,
        }
    }

    fn alu(&mut self, op: u8, value: u8) {
        match op {
            0 => self.add(value),
            1 => self.adc(value),
            2 => self.sub(value),
            3 => self.sbc(value),
            4 => self.and(value),
            5 => self.xor(value),
            6 => self.or(value),
            7 => self.cp(value),
            _ => unreachable!("alu opcode out of range {op}"),
        }
    }

    /// The adder behind ADD and ADC. `carry_in` is the C flag for ADC and
    /// false for ADD -- the arithmetic is otherwise identical.
    fn add_with_carry(&mut self, value: u8, carry_in: bool) {
        let a = self.regs.a;
        let c = carry_in as u8;

        let result = a.wrapping_add(value).wrapping_add(c);

        // H is the carry out of bit 3: add the low nibbles and see if the
        // total needs a fifth bit. Max is 15 + 15 + 1 = 31, so no overflow.
        let half = (a & 0x0F) + (value & 0x0F) + c > 0x0F;
        // C is the carry out of bit 7. Widening to u16 keeps the sum that
        // does not fit in a u8, which is exactly what we are testing for.
        let full = a as u16 + value as u16 + c as u16 > 0xFF;

        self.regs.a = result;
        self.regs.set_flags(result == 0, false, half, full);
    }

    /// The subtractor behind SUB, SBC and CP. Returns the result and leaves A
    /// alone, so CP can set the flags and throw the difference away.
    fn sub_with_carry(&mut self, value: u8, carry_in: bool) -> u8 {
        let a = self.regs.a;
        let c = carry_in as u8;

        let result = a.wrapping_sub(value).wrapping_sub(c);

        // Borrowing is the mirror of carrying: H is set when the low nibble
        // we are taking away is bigger than the one we have.
        let half = (a & 0x0F) < (value & 0x0F) + c;
        let full = (a as u16) < value as u16 + c as u16;

        self.regs.set_flags(result == 0, true, half, full);
        result
    }

    fn add(&mut self, value: u8) {
        self.add_with_carry(value, false);
    }

    fn adc(&mut self, value: u8) {
        self.add_with_carry(value, self.regs.flag_c());
    }

    fn sub(&mut self, value: u8) {
        self.regs.a = self.sub_with_carry(value, false);
    }

    fn sbc(&mut self, value: u8) {
        self.regs.a = self.sub_with_carry(value, self.regs.flag_c());
    }

    fn and(&mut self, value: u8) {
        self.regs.a &= value;
        // AND is the odd one out: it always sets H.
        self.regs.set_flags(self.regs.a == 0, false, true, false);
    }

    fn xor(&mut self, value: u8) {
        self.regs.a ^= value;
        self.regs.set_flags(self.regs.a == 0, false, false, false);
    }

    fn or(&mut self, value: u8) {
        self.regs.a |= value;
        self.regs.set_flags(self.regs.a == 0, false, false, false);
    }

    /// CP is SUB that keeps the flags and throws the difference away.
    fn cp(&mut self, value: u8) {
        self.sub_with_carry(value, false);
    }

    pub fn write_trace<W: Write>(&self, bus: &Bus, out: &mut W) -> io::Result<()> {
        let reg = self.regs;
        let pc = reg.pc;
        writeln!(
            out,
            "A:{:02X} F:{:02X} B:{:02X} C:{:02X} D:{:02X} E:{:02X} H:{:02X} L:{:02X} \
            SP:{:04X} PC:{:04X} PCMEM:{:02X},{:02X},{:02X},{:02X}",
            reg.a, reg.f(), reg.b, reg.c,
            reg.d, reg.e, reg.h, reg.l,
            reg.sp, pc,
            bus.read(pc), bus.read(pc.wrapping_add(1)),
            bus.read(pc.wrapping_add(2)), bus.read(pc.wrapping_add(3))
        )
    }

    fn jr(&mut self, bus: &mut Bus) -> u32 {
        let value = self.fetch8(bus) as i8 as u16;
        self.regs.pc = self.regs.pc.wrapping_add(value);
        12
    }

    fn jrcc(&mut self, bus: &mut Bus, y: u8) -> u32 {
        let taken = self.condition(y - 4);
        let e = self.fetch8(bus) as i8 as u16;
        if taken {
            self.regs.pc = self.regs.pc.wrapping_add(e);
            12
        } else {
            8
        }
    }

    fn condition(&self, cc: u8) -> bool {
        match cc {
            0 => !self.regs.flag_z(),
            1 => self.regs.flag_z(),
            2 => !self.regs.flag_c(),
            3 => self.regs.flag_c(),
            _ => unreachable!(),
        }
    }

    fn ld_rp_n16(&mut self, bus: &mut Bus, p: u8) -> u32 {
        let value = self.fetch16(bus);
        self.set_r16(R16::from_code_sp(p), value);
        12
    }

    fn set_r16(&mut self, r16: R16, value: u16) {
        match r16 {
            R16::BC => self.regs.set_bc(value),
            R16::DE => self.regs.set_de(value),
            R16::HL => self.regs.set_hl(value),
            R16::AF => self.regs.set_af(value),
            R16::SP => self.regs.sp = value,
        };
    }

    fn get_r16(&self, r16: R16) -> u16{
        match r16 {
            R16::BC => self.regs.bc(),
            R16::DE => self.regs.de(),
            R16::HL => self.regs.hl(),
            R16::AF => self.regs.af(),
            R16::SP => self.regs.sp,
        }
    }

    fn add_hl_rp(&mut self, p: u8) -> u32 {
        let hl = self.regs.hl();
        let v = self.get_r16(R16::from_code_sp(p));
        let (result, carry) = hl.overflowing_add(v);
        let half = (hl & 0x0FFF) + (v & 0x0FFF) > 0x0FFF;
        self.regs.set_flags(self.regs.flag_z(), false, half, carry);
        self.regs.set_hl(result);
        8
    }

    fn ld_r8_n8(&mut self, bus: &mut Bus, y: u8) -> u32 {
        let value = self.fetch8(bus);
        self.set_r8(R8::from_code(y), bus, value);
        if y == 6 { 12 } else { 8 }
    }

    fn inc_r8(&mut self, bus: &mut Bus, y: u8) -> u32 {
        let r8 = R8::from_code(y);
        let v = self.get_r8(r8, bus);
        let res = v.wrapping_add(1);
        self.set_r8(r8, bus, res);
        self.regs.set_flags(res == 0, false, (v & 0xF) == 0xF, self.regs.flag_c());
        if y == 6 { 12 } else { 4 }
    }
    fn dec_r8(&mut self, bus: &mut Bus, y: u8) -> u32 {
        let r8 = R8::from_code(y);
        let v = self.get_r8(r8, bus);
        let res = v.wrapping_sub(1);
        self.set_r8(r8, bus, res);
        self.regs.set_flags(res == 0, true, (v & 0xF) == 0x0, self.regs.flag_c());
        if y == 6 { 12 } else { 4 }
    }

    fn inc_rp(&mut self, p: u8) -> u32 {
        let r16 = R16::from_code_sp(p);
        self.set_r16(r16, self.get_r16(r16).wrapping_add(1));
        8
    }

    fn dec_rp(&mut self, p: u8) -> u32 {
        let r16 = R16::from_code_sp(p);
        self.set_r16(r16, self.get_r16(r16).wrapping_sub(1));
        8
    }

    fn ld_r16_a(&mut self, bus: &mut Bus, p: u8, q: u8) -> u32 {
        let addr = match p {
            0 => self.regs.bc(),
            1 => self.regs.de(),
            _ => self.regs.hl(),
        };
        if q == 0 {
            bus.write(addr, self.regs.a);
        } else {
            self.regs.a = bus.read(addr);
        }

        match p {
            2 => self.regs.set_hl(addr.wrapping_add(1)),
            3 => self.regs.set_hl(addr.wrapping_sub(1)),
            _ => {}
        };
        8
    }

    fn ldh_n_a(&mut self, bus: &mut Bus, y: u8) -> u32 {
        let addr = self.fetch8(bus) as u16 + 0xFF00;
        if y == 4 {
            bus.write(addr, self.regs.a);
        } else {
            self.regs.a = bus.read(addr);
        }
        12
    }

    fn ld_a_c(&mut self, bus: &mut Bus, y: u8) -> u32 {
        let addr = self.regs.c as u16 + 0xFF00;
        if y == 4 {
            bus.write(addr, self.regs.a);
        } else {
            self.regs.a = bus.read(addr);
        }
        8
    }

    fn ld_n16_a(&mut self, bus: &mut Bus, y: u8) -> u32 {
        let addr = self.fetch16(bus);
        if y == 5 {
            bus.write(addr, self.regs.a);
        } else {
            self.regs.a = bus.read(addr);
        }
        16
    }

    fn ret(&mut self, bus: &mut Bus) -> u32 {
        let value = self.pop16(bus);
        self.regs.pc = value;
        16
    }

    fn ret_cc(&mut self, bus: &mut Bus, y: u8) -> u32 {
        let taken = self.condition(y);
        if taken {
            self.regs.pc = self.pop16(bus);
            20
        } else {
            8
        }
    }

    fn call_n16(&mut self, bus: &mut Bus) -> u32 {
        let addr = self.fetch16(bus);
        self.push16(bus, self.regs.pc);
        self.regs.pc = addr;
        24
    }

    fn call_cc(&mut self, bus: &mut Bus, y: u8) -> u32 {
        let addr = self.fetch16(bus);
        let taken = self.condition(y);
        if taken {
            self.push16(bus, self.regs.pc);
            self.regs.pc = addr;
            24
        } else {
            12
        }
    }

    fn jp_cc(&mut self, bus: &mut Bus, y: u8) -> u32 {
        let taken = self.condition(y);
        let addr = self.fetch16(bus);
        if taken {
            self.regs.pc = addr;
            16
        } else {
            12
        }
    }

    fn jp_hl(&mut self) -> u32 {
        self.regs.pc = self.regs.hl();
        4
    }

    fn rst(&mut self, bus: &mut Bus, y: u8) -> u32 {
        self.push16(bus, self.regs.pc);
        self.regs.pc = (y * 8) as u16;
        16
    }

    fn rlca(&mut self) -> u32 {
        self.regs.a = self.regs.a.rotate_left(1);
        self.regs.set_flags(false,false,false,(self.regs.a & 1) == 1);
        4
    }

    fn rrca(&mut self) -> u32 {
        self.regs.a = self.regs.a.rotate_right(1);
        self.regs.set_flags(false,false,false,(self.regs.a >> 7) == 1);
        4
    }

    fn rla(&mut self) -> u32 {
        let old_c = self.regs.flag_c() as u8;
        let old_a = self.regs.a;
        self.regs.a = (old_a << 1) | old_c;
        self.regs.set_flags(false,false,false,(old_a >> 7) & 1 == 1);
        4
    }

    fn rra(&mut self) -> u32 {
        let old_c = self.regs.flag_c() as u8;
        let old_a = self.regs.a;
        self.regs.a = (old_a >> 1) | (old_c << 7);
        self.regs.set_flags(false,false,false,old_a  & 1 == 1);
        4
    }

    fn daa(&mut self) -> u32 {
        let n_flag = self.regs.flag_n();
        let h_flag = self.regs.flag_h();
        let mut c_flag = self.regs.flag_c();
        let mut adjustment = 0;
        if !n_flag {
            if h_flag || (self.regs.a & 0x0F) > 0x09 {
                adjustment |= 0x06;
            }
            if c_flag || self.regs.a > 0x99 {
                adjustment |= 0x60;
                c_flag = true;
            }
        } else {
            if h_flag {
                adjustment |= 0x06;
            }
            if c_flag {
                adjustment |= 0x60;
            }
        }

        if !n_flag {
            self.regs.a = self.regs.a.wrapping_add(adjustment);
        } else {
            self.regs.a = self.regs.a.wrapping_sub(adjustment);
        }

        let z_flag = self.regs.a == 0;
        self.regs.set_flags(z_flag, n_flag, false, c_flag);
        4
    }

    fn cpl(&mut self) -> u32 {
        self.regs.a = !self.regs.a;
        self.regs.set_flags(
            self.regs.flag_z(),
            true,
            true,
            self.regs.flag_c()
        );
        4
    }

    fn scf(&mut self) -> u32 {
        self.regs.set_flags(
            self.regs.flag_z(),
            false,
            false,
            true
        );
        4
    }

    fn ccf(&mut self) -> u32 {
        self.regs.set_flags(
            self.regs.flag_z(),
            false,
            false,
            !self.regs.flag_c()
        );
        4
    }

    fn cb(&mut self, bus: &mut Bus) -> u32 {
        let cb = self.fetch8(bus);
        let cx = cb >> 6;
        let cy = (cb >> 3) & 7;
        let cz = cb & 7;
        match cx {
            0 => self.rot(bus, cy, cz),
            1 => self.bit(bus, cy, cz),
            2 => self.res(bus, cy, cz),
            3 => self.set_bit(bus, cy, cz),
            _ => unreachable!()
        }
    }

    fn rot(&mut self, bus: &mut Bus, y: u8, z: u8) -> u32 {
        let r8 = R8::from_code(z);
        let v = self.get_r8(r8, bus);
        let old_c = self.regs.flag_c() as u8;
        let (result, carry) = match y {
            0 => (v.rotate_left(1),          v & 0x80 != 0), // RLC
            1 => (v.rotate_right(1),         v & 0x01 != 0), // RRC
            2 => ((v << 1) | old_c,             v & 0x80 != 0), // RL
            3 => ((v >> 1) | (old_c << 7),      v & 0x01 != 0), // RR
            4 => (v << 1,                       v & 0x80 != 0), // SLA
            5 => (((v as i8) >> 1) as u8,       v & 0x01 != 0), // SRA
            6 => (v.rotate_left(4),          false),         // SWAP
            7 => (v >> 1,                       v & 0x01 != 0), // SRL
            _ => unreachable!(),
        };
        self.set_r8(r8, bus, result);
        self.regs.set_flags(result == 0, false, false, carry);
        if z == 6 { 16 } else { 8 }
    }

    fn bit(&mut self, bus: &mut Bus, y: u8, z: u8) -> u32 {
        let r8 = self.get_r8(R8::from_code(z), bus);
        let z_flag = r8 & (1 << y) == 0;
        self.regs.set_flags(z_flag, false, true, self.regs.flag_c());
        if z == 6 { 12 } else { 8 }
    }

    fn res(&mut self, bus: &mut Bus, y: u8, z: u8) -> u32 {
        let r8 = R8::from_code(z);
        let value = self.get_r8(r8, bus);
        self.set_r8(r8, bus, value & !(1 << y));
        if z == 6 { 16 } else { 8 }
    }

    fn set_bit(&mut self, bus: &mut Bus, y: u8, z: u8) -> u32 {
        let r8 = R8::from_code(z);
        let value = self.get_r8(r8, bus);
        self.set_r8(r8, bus, value | (1 << y));
        if z == 6 { 16 } else { 8 }
    }

    fn ld_n16_sp(&mut self, bus: &mut Bus) -> u32 {
        let addr = self.fetch16(bus);
        let sp = self.regs.sp;
        bus.write(addr, (sp & 0xFF) as u8);
        bus.write(addr.wrapping_add(1), (sp >> 8) as u8);
        20
    }

    fn ld_sp_hl(&mut self) -> u32 {
        self.regs.sp = self.regs.hl();
        8
    }

    fn add_sp_e(&mut self, bus: &mut Bus) -> u32 {
        let e = self.fetch8(bus) as i8 as i16 as u16; // sign-extend
        let sp = self.regs.sp;

        self.regs.sp = sp.wrapping_add(e);
        self.regs.set_flags(false, false, (sp & 0x0F) + (e & 0x0F) > 0x0F, (sp & 0xFF) + (e & 0xFF) > 0xFF);
        16
    }

    fn ld_hl_spe(&mut self, bus: &mut Bus) -> u32 {
        let e = self.fetch8(bus) as i8 as i16 as u16;
        let sp = self.regs.sp;

        self.regs.set_hl(sp.wrapping_add(e));
        self.regs.set_flags(false, false, (sp & 0x0F) + (e & 0x0F) > 0x0F, (sp & 0xFF) + (e & 0xFF) > 0xFF);
        12
    }
}

#[derive(Clone, Copy)]
enum R8 { B, C, D, E, H, L, HlMem, A }

impl R8 {
    fn from_code(code: u8) -> R8 {
        match code {
            0 => R8::B,
            1 => R8::C,
            2 => R8::D,
            3 => R8::E,
            4 => R8::H,
            5 => R8::L,
            6 => R8::HlMem,
            7 => R8::A,
            _ => unreachable!("R8 code out of range: {code}"),
        }
    }
}

#[derive(Clone, Copy)]
enum R16 { BC, DE, HL, SP, AF }
impl R16 {
    fn from_code_sp(code: u8) -> R16 {
        match code {
            0 => R16::BC,
            1 => R16::DE,
            2 => R16::HL,
            3 => R16::SP,
            _ => unreachable!("R16 code out of range: {code}"),
        }
    }

    fn from_code_af(code: u8) -> R16 {
        match code {
            0 => R16::BC,
            1 => R16::DE,
            2 => R16::HL,
            3 => R16::AF,
            _ => unreachable!("R16 code out of range: {code}"),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartridge::{fake_rom, Cartridge};

    const VBLANK: u8 = 0;
    const TIMER: u8 = 2;

    /// A bus over a blank 32 KiB cartridge. Address 0x0100, where post_boot
    /// leaves PC, holds 0x00 -- so an undispatched step executes a NOP.
    fn test_bus() -> Bus {
        Bus::new(Cartridge::from_bytes(fake_rom("TEST", 0x00, 0x00, 0x00)).unwrap())
    }

    /// A CPU with interrupts unmasked and `sources` already requested.
    fn armed(bus: &mut Bus, sources: &[u8]) -> Cpu {
        let mut cpu = Cpu::new();
        cpu.ime = true;
        bus.write(0xFFFF, 0xFF); // IE: every source enabled
        for &bit in sources {
            bus.request_interrupt(bit);
        }
        cpu
    }

    #[test]
    fn dispatch_vectors_to_the_right_handler() {
        let mut bus = test_bus();
        let mut cpu = armed(&mut bus, &[TIMER]);

        let cycles = cpu.step(&mut bus);

        assert_eq!(cpu.regs.pc, 0x0050, "the timer vector is 0x40 + 2 * 8");
        assert_eq!(cycles, 20);
    }

    #[test]
    fn dispatch_acknowledges_the_source_and_masks_further_interrupts() {
        let mut bus = test_bus();
        let mut cpu = armed(&mut bus, &[TIMER]);

        cpu.step(&mut bus);

        assert!(!cpu.ime, "IME must be clear so the handler is not interrupted");
        assert_eq!(
            bus.pending_interrupts() & (1 << TIMER),
            0,
            "the serviced request must be cleared from IF, not left set"
        );
    }

    #[test]
    fn dispatch_pushes_the_return_address() {
        let mut bus = test_bus();
        let mut cpu = armed(&mut bus, &[TIMER]);
        let before = cpu.regs.pc;

        cpu.step(&mut bus);

        assert_eq!(cpu.regs.sp, 0xFFFE - 2, "two bytes should be on the stack");
        assert_eq!(cpu.pop16(&mut bus), before, "RETI must come back to where we left");
    }

    #[test]
    fn vblank_outranks_the_timer() {
        let mut bus = test_bus();
        let mut cpu = armed(&mut bus, &[VBLANK, TIMER]);

        cpu.step(&mut bus);

        assert_eq!(cpu.regs.pc, 0x0040, "the lowest set bit wins");
        assert_ne!(
            bus.pending_interrupts() & (1 << TIMER),
            0,
            "the timer request must stay pending for the next dispatch"
        );
    }

    #[test]
    fn a_request_waits_while_ime_is_clear() {
        let mut bus = test_bus();
        let mut cpu = armed(&mut bus, &[TIMER]);
        cpu.ime = false; // DI, or inside another handler

        cpu.step(&mut bus);

        assert_eq!(cpu.regs.pc, 0x0101, "the NOP at 0x0100 should have run instead");
        assert_ne!(
            bus.pending_interrupts() & (1 << TIMER),
            0,
            "the request is deferred, not discarded"
        );
    }

    #[test]
    fn ie_gates_dispatch() {
        let mut bus = test_bus();
        let mut cpu = armed(&mut bus, &[TIMER]);
        bus.write(0xFFFF, 0x00); // IE: nothing enabled

        cpu.step(&mut bus);

        assert_eq!(cpu.regs.pc, 0x0101, "a request with IE clear must not dispatch");
    }

    #[test]
    fn will_execute_reports_whether_an_opcode_runs() {
        let mut bus = test_bus();
        let cpu = armed(&mut bus, &[TIMER]);

        // gameboy-doctor wants one trace line per executed opcode, and a
        // dispatch executes none.
        assert!(!cpu.will_execute(&bus));

        let mut bus = test_bus();
        let mut cpu = armed(&mut bus, &[]);
        assert!(cpu.will_execute(&bus));

        cpu.halted = true;
        assert!(!cpu.will_execute(&bus));
    }
}
