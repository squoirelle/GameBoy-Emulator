/// The joypad is one register at 0xFF00, and everything about it is
/// active-low: the CPU writes a 0 to pick a row of buttons, and reads a 0 back
/// for each button in that row that is held down.
pub struct Joypad {
    /// Bits 4-5 exactly as the CPU wrote them. 0 selects that row.
    select: u8,
    /// One bit per button, 1 = held. Low nibble is the d-pad, high nibble the
    /// action buttons -- the same order each row reports them in.
    pressed: u8,
}

#[derive(Clone, Copy)]
pub enum Button {
    Right,
    Left,
    Up,
    Down,
    A,
    B,
    Select,
    Start,
}

impl Button {
    fn mask(self) -> u8 {
        match self {
            Button::Right => 0x01,
            Button::Left => 0x02,
            Button::Up => 0x04,
            Button::Down => 0x08,
            Button::A => 0x10,
            Button::B => 0x20,
            Button::Select => 0x40,
            Button::Start => 0x80,
        }
    }
}

impl Joypad {
    pub fn new() -> Self {
        // Out of reset nothing is selected and nothing is held.
        Joypad { select: 0x30, pressed: 0x00 }
    }

    pub fn set(&mut self, button: Button, down: bool) {
        if down {
            self.pressed |= button.mask();
        } else {
            self.pressed &= !button.mask();
        }
    }

    /// Only bits 4-5 are writable; the rest belong to the hardware.
    pub fn write(&mut self, value: u8) {
        self.select = value & 0x30;
    }

    pub fn read(&self) -> u8 {
        let mut buttons = 0x0F; // all released

        if self.select & 0x10 == 0 {
            buttons &= !(self.pressed & 0x0F); // d-pad row
        }
        if self.select & 0x20 == 0 {
            buttons &= !(self.pressed >> 4); // action row
        }

        // Bits 6-7 are unimplemented and always read as 1. Getting this wrong
        // -- returning 0x00 -- reads as every button held, and games race
        // through their menus.
        0xC0 | self.select | buttons
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Selects a row the way a game does: write 0 to that row's bit.
    const DPAD: u8 = 0x20; // bit 4 clear -> d-pad selected
    const ACTION: u8 = 0x10; // bit 5 clear -> action buttons selected

    #[test]
    fn nothing_held_reads_all_ones() {
        let mut pad = Joypad::new();
        pad.write(DPAD);
        assert_eq!(pad.read(), 0xEF, "a released button must read as 1, not 0");
    }

    #[test]
    fn a_held_button_reads_as_zero() {
        let mut pad = Joypad::new();
        pad.write(DPAD);
        pad.set(Button::Right, true);
        assert_eq!(pad.read() & 0x01, 0);

        pad.set(Button::Right, false);
        assert_eq!(pad.read() & 0x01, 0x01);
    }

    #[test]
    fn each_row_reports_only_its_own_buttons() {
        let mut pad = Joypad::new();
        pad.set(Button::Right, true); // d-pad bit 0
        pad.set(Button::B, true); // action bit 1

        pad.write(DPAD);
        assert_eq!(pad.read() & 0x0F, 0x0E, "only Right should read low");

        pad.write(ACTION);
        assert_eq!(pad.read() & 0x0F, 0x0D, "only B should read low");
    }

    #[test]
    fn selecting_both_rows_ands_them_together() {
        let mut pad = Joypad::new();
        pad.set(Button::Right, true); // d-pad bit 0
        pad.set(Button::B, true); // action bit 1

        pad.write(0x00); // both rows selected at once
        assert_eq!(pad.read() & 0x0F, 0x0C, "both buttons should read low");
    }

    #[test]
    fn the_top_two_bits_are_always_set() {
        let mut pad = Joypad::new();
        for select in [0x00, 0x10, 0x20, 0x30] {
            pad.write(select);
            assert_eq!(pad.read() & 0xC0, 0xC0);
            assert_eq!(pad.read() & 0x30, select, "the selection must read back");
        }
    }
}
