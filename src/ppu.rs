use crate::{HEIGHT, WIDTH};

const CYCLES_PER_LINE: u32 = 456;
const LINES_PER_FRAME: u8 = 154;
const VBLANK_LINE: u8 = 144;

// Within a visible line: OAM scan, then drawing, then HBlank fills the rest.
const OAM_CYCLES: u32 = 80;
const DRAW_CYCLES: u32 = 172;

pub struct Ppu {
    // Wide enough that a large tick cannot silently truncate on the way in.
    counter: u32,
    pub lcdc: u8,
    pub stat: u8,
    pub scy : u8,
    pub scx : u8,
    pub ly: u8,
    pub lyc: u8,
    pub bgp: u8,
    pub wy: u8,
    pub wx: u8,
    pub framebuffer: [u8; WIDTH * HEIGHT],
    pub vram: [u8; 0x2000],
    bg_ids: [u8; 160],
    /// Edge detection for the STAT interrupt line.
    prev_stat_line: bool,
    pub oam: [u8; 0xA0],
    pub obp0: u8,
    pub obp1: u8,
}

impl Ppu {
    pub(crate) fn new() -> Self {
        Ppu {
            counter: 0,
            lcdc: 0x91,
            stat: 0,
            scy: 0,
            scx: 0,
            ly: 0,
            lyc: 0,
            bgp: 0xFC,
            wy: 0,
            wx: 0,
            framebuffer: [0; WIDTH * HEIGHT],
            vram: [0; 0x2000],
            bg_ids: [0; 160],
            prev_stat_line: false,
            oam: [0; 0xA0],
            obp0: 0,
            obp1: 0,
        }
    }

    /// LCDC has to go through here so switching the LCD off can reset the
    /// schedule, which a plain field write could not do.
    pub fn set_lcdc(&mut self, value: u8) {
        let was_on = self.lcdc & 0x80 != 0;
        self.lcdc = value;

        if was_on && value & 0x80 == 0 {
            // The panel goes blank and the schedule restarts from line 0. Games
            // switch the LCD off precisely so they can rewrite VRAM without a
            // frame being drawn from it half-finished.
            self.ly = 0;
            self.counter = 0;
            self.framebuffer.fill(0);
            self.bg_ids.fill(0);
        }
    }

    /// Which of the four states the PPU is in right now. Bits 0-1 of STAT, and
    /// derived rather than stored -- it is a function of LY and the counter.
    fn mode(&self) -> u8 {
        if self.ly >= VBLANK_LINE {
            1 // VBlank
        } else if self.counter < OAM_CYCLES {
            2 // scanning OAM
        } else if self.counter < OAM_CYCLES + DRAW_CYCLES {
            3 // drawing
        } else {
            0 // HBlank
        }
    }

    /// STAT bits 0-2 belong to the hardware, so they are composed on read
    /// rather than stored. Bit 7 does not exist and always reads as 1.
    pub fn stat(&self) -> u8 {
        let coincidence = if self.ly == self.lyc { 0x04 } else { 0x00 };
        0x80 | (self.stat & 0x78) | coincidence | self.mode()
    }

    /// Only the four interrupt selects in bits 3-6 are writable.
    pub fn set_stat(&mut self, value: u8) {
        self.stat = value & 0x78;
    }

    /// The four conditions STAT can interrupt on, ORed into one line. The
    /// interrupt fires on its rising edge, so a mode change while an already
    /// selected condition holds raises nothing -- which is what stops a
    /// selected HBlank from firing 144 times a frame for one enable.
    fn stat_line(&self) -> bool {
        let mode = self.mode();
        (self.stat & 0x08 != 0 && mode == 0)
            || (self.stat & 0x10 != 0 && mode == 1)
            || (self.stat & 0x20 != 0 && mode == 2)
            || (self.stat & 0x40 != 0 && self.ly == self.lyc)
    }

    /// Advances the schedule and returns the interrupts to request, as IF bits:
    /// bit 0 VBlank, bit 1 STAT.
    pub fn tick(&mut self, cycles: u32) -> u8 {
        // With the LCD off the PPU is idle: LY stays at 0, nothing is drawn,
        // and nothing is raised.
        if self.lcdc & 0x80 == 0 {
            return 0;
        }

        self.counter += cycles;

        let mut requested = 0u8;
        while self.counter >= CYCLES_PER_LINE {
            self.counter -= CYCLES_PER_LINE;
            let finished = self.ly;
            self.ly = (self.ly + 1) % LINES_PER_FRAME;
            if finished < VBLANK_LINE {
                self.render_line(finished);
            }
            if self.ly == VBLANK_LINE {
                requested |= 0x01;
            }
        }

        let line = self.stat_line();
        if line && !self.prev_stat_line {
            requested |= 0x02;
        }
        self.prev_stat_line = line;

        requested
    }

    fn render_background(&mut self, line: u8) {
        // With the background off the line is blank white, and its colour ids
        // must be cleared too or sprite priority tests the previous line's.
        if self.lcdc & 0x01 == 0 {
            let base = line as usize * 160;
            self.framebuffer[base..base + 160].fill(0);
            self.bg_ids.fill(0);
            return;
        }
        let map_base = if self.lcdc & 0x08 != 0 { 0x1C00 } else { 0x1800 };
        let signed   = self.lcdc & 0x10 == 0;

        let map_y     = line.wrapping_add(self.scy);   // wraps in the 256x256 map
        let tile_row  = (map_y / 8) as usize;
        let pixel_row = (map_y % 8) as usize;

        for x in 0..160u8 {
            let map_x     = x.wrapping_add(self.scx);
            let tile_col  = (map_x / 8) as usize;
            let pixel_col = (map_x % 8) as usize;
            let index = self.vram[map_base + tile_row * 32 + tile_col];
            let tile  = if signed {
                (0x1000 + (index as i8 as isize) * 16) as usize
            } else {
                index as usize * 16
            };

            let lo = self.vram[tile + pixel_row * 2];
            let hi = self.vram[tile + pixel_row * 2 + 1];
            let bit = 7 - pixel_col;
            let id  = ((hi >> bit) & 1) << 1 | ((lo >> bit) & 1);

            self.framebuffer[line as usize * 160 + x as usize] = (self.bgp >> (id * 2)) & 3;
            self.bg_ids[x as usize] = id;
        }
    }

    fn render_line(&mut self, line: u8) {
        self.render_background(line);
        if self.lcdc & 0x02 != 0 {          // LCDC bit 1: sprites enabled
            self.render_sprites(line);
        }
    }

    fn render_sprites(&mut self, line: u8) {
        let height: i16 = if self.lcdc & 0x04 != 0 { 16 } else { 8 };
        let mut chosen: Vec<usize> = Vec::new();

        for i in 0..40 {
            let top = self.oam[i * 4] as i16 - 16;    // stored Y is offset by 16
            if (line as i16) >= top && (line as i16) < top + height && chosen.len() < 10 {
                chosen.push(i);
            }
        }
        chosen.sort_by_key(|&i| (self.oam[i * 4 + 1], i));
        chosen.reverse();
        for i in chosen {
            let y = self.oam[i * 4] as i16 - 16;
            let x = self.oam[i * 4 + 1] as i16 - 8;
            let attr = self.oam[i * 4 + 3];
            let mut tile = self.oam[i * 4 + 2];
            if height == 16 { tile &= 0xFE; }        // 8x16 ignores the low bit

            let mut row = line as i16 - y;
            if attr & 0x40 != 0 { row = height - 1 - row; }   // Y flip

            let addr = tile as usize * 16 + row as usize * 2; // always 0x8000, unsigned
            let lo = self.vram[addr];
            let hi = self.vram[addr + 1];

            let palette = if attr & 0x10 != 0 { self.obp1 } else { self.obp0 };

            for col in 0..8i16 {
                let sx = x + col;
                if sx < 0 || sx >= 160 { continue; }

                let bit = if attr & 0x20 != 0 { col } else { 7 - col };   // X flip
                let id = ((hi >> bit) & 1) << 1 | ((lo >> bit) & 1);

                if id == 0 { continue; }                                  // transparent
                if attr & 0x80 != 0 && self.bg_ids[sx as usize] != 0 { continue; }

                self.framebuffer[line as usize * 160 + sx as usize] = (palette >> (id * 2)) & 3;
            }
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    /// A PPU with the LCD on and the background off, so ticking it does not
    /// touch VRAM.
    fn ppu() -> Ppu {
        let mut ppu = Ppu::new();
        ppu.set_lcdc(0x80);
        ppu
    }

    #[test]
    fn ly_walks_the_frame_and_vblank_fires_once() {
        let mut ppu = ppu();
        let mut vblanks = 0;

        for _ in 0..(CYCLES_PER_LINE * LINES_PER_FRAME as u32 / 4) {
            if ppu.tick(4) & 0x01 != 0 {
                vblanks += 1;
            }
        }

        assert_eq!(ppu.ly, 0, "one frame is exactly 154 lines");
        assert_eq!(vblanks, 1, "VBlank is raised once a frame, on reaching line 144");
    }

    #[test]
    fn the_lcd_being_off_stops_everything() {
        let mut ppu = ppu();
        ppu.tick(CYCLES_PER_LINE * 3);
        assert_eq!(ppu.ly, 3);

        ppu.set_lcdc(0x00); // LCD off
        assert_eq!(ppu.ly, 0, "switching the LCD off restarts the schedule");

        for _ in 0..1000 {
            assert_eq!(ppu.tick(CYCLES_PER_LINE), 0, "an idle PPU raises nothing");
        }
        assert_eq!(ppu.ly, 0, "LY must not advance with the LCD off");
    }

    #[test]
    fn mode_follows_the_position_within_a_line() {
        let mut ppu = ppu();
        assert_eq!(ppu.stat() & 3, 2, "a line opens scanning OAM");

        ppu.tick(OAM_CYCLES);
        assert_eq!(ppu.stat() & 3, 3, "then draws");

        ppu.tick(DRAW_CYCLES);
        assert_eq!(ppu.stat() & 3, 0, "then idles until the line ends");

        ppu.tick(CYCLES_PER_LINE * VBLANK_LINE as u32);
        assert_eq!(ppu.stat() & 3, 1, "lines 144 and up are VBlank");
    }

    #[test]
    fn stat_reports_the_coincidence_flag_and_hides_the_unused_bit() {
        let mut ppu = ppu();
        ppu.lyc = 2;

        assert_eq!(ppu.stat() & 0x04, 0);
        assert_eq!(ppu.stat() & 0x80, 0x80, "bit 7 does not exist and reads as 1");

        ppu.tick(CYCLES_PER_LINE * 2);
        assert_eq!(ppu.ly, 2);
        assert_eq!(ppu.stat() & 0x04, 0x04, "LY == LYC must show in bit 2");
    }

    #[test]
    fn only_the_interrupt_selects_are_writable() {
        let mut ppu = ppu();
        ppu.set_stat(0xFF);
        assert_eq!(ppu.stat() & 0x78, 0x78, "bits 3-6 are the writable ones");

        ppu.set_stat(0x00);
        assert_eq!(ppu.stat() & 0x78, 0x00);
        assert_eq!(ppu.stat() & 3, 2, "the mode is the hardware's, not the write's");
    }

    #[test]
    fn the_stat_interrupt_fires_on_a_rising_edge_only() {
        let mut ppu = ppu();
        ppu.lyc = 5;
        ppu.set_stat(0x40); // interrupt on LY == LYC

        let mut fires = 0;
        for _ in 0..(LINES_PER_FRAME as u32 * 4) {
            // Quarter-lines, so the condition is sampled repeatedly while it
            // holds. A level-triggered implementation would fire every time.
            if ppu.tick(CYCLES_PER_LINE / 4) & 0x02 != 0 {
                fires += 1;
            }
        }
        assert_eq!(fires, 1, "one frame passes LY == LYC once");
    }

    #[test]
    fn a_deselected_condition_raises_nothing() {
        let mut ppu = ppu();
        ppu.lyc = 5;
        ppu.set_stat(0x00); // nothing selected

        for _ in 0..(LINES_PER_FRAME as u32) {
            assert_eq!(ppu.tick(CYCLES_PER_LINE) & 0x02, 0);
        }
    }
}
