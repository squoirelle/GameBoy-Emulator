use crate::{HEIGHT, WIDTH};

const CYCLES_PER_LINE: u16 = 456;
const LINES_PER_FRAME: u8 = 154;
const VBLANK_LINE: u8 = 144;

pub struct Ppu {
    counter: u16,
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
            oam: [0; 0xA0],
            obp0: 0,
            obp1: 0,
        }
    }

    pub fn tick(&mut self, cycles: u32) -> bool {
        self.counter += cycles as u16;

        let mut vblank = false;
        while self.counter >= CYCLES_PER_LINE {
            self.counter -= CYCLES_PER_LINE;
            let finished = self.ly;
            self.ly = (self.ly + 1) % LINES_PER_FRAME;
            if finished < VBLANK_LINE {
                self.render_line(finished);
            }
            if self.ly == VBLANK_LINE {
                vblank = true;
            }
        }
        vblank
    }

    fn render_background(&mut self, line: u8) {
        if self.lcdc & 0x01 == 0 {
            let base = line as usize * 160;
            self.framebuffer[base..base + 160].fill(0);
            self.bg_ids.fill(0);
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

