pub struct Timer {
    counter: u16,
    pub tima: u8,
    pub tma: u8,
    pub tac: u8,
}

impl Timer {
    pub fn new() -> Self {
        Timer { counter: 0, tima: 0u8, tac: 0u8, tma: 0u8 }
    }

    pub fn tick(&mut self, cycles: u32) -> bool{
        let mut fired = false;
        for _ in 0..cycles {
            let before = self.timer_bit();
            self.counter = self.counter.wrapping_add(1);
            if before && !self.timer_bit() {
                fired |= self.increment_tima();
            }
        }
        fired
    }

    pub fn div(&self) -> u8 {
        (self.counter >> 8) as u8
    }

    pub fn reset_div(&mut self) {
        self.counter = 0;
    }

    fn timer_bit(&self) -> bool {
        let bit = match self.tac & 0b11 { 0 => 9, 1 => 3, 2 => 5, _ => 7 };
        self.tac & 0b100 != 0 && (self.counter >> bit) & 1 != 0
    }

    fn increment_tima(&mut self) -> bool {
        let (v, overflow) = self.tima.overflowing_add(1);
        self.tima = if overflow { self.tma } else { v };
        overflow
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn div_is_the_counters_high_byte() {
        let mut timer = Timer::new();

        timer.tick(255);
        assert_eq!(timer.div(), 0x00, "255 cycles is not a whole DIV step");

        timer.tick(1);
        assert_eq!(timer.div(), 0x01);

        timer.tick(255);
        assert_eq!(timer.div(), 0x01, "DIV holds until the next multiple of 256");

        timer.tick(1);
        assert_eq!(timer.div(), 0x02);
    }

    #[test]
    fn writing_div_clears_the_whole_counter() {
        let mut timer = Timer::new();

        // Stop part way through a DIV step, so the low half of the counter --
        // the half no register exposes -- holds a value.
        timer.tick(200);
        timer.reset_div();
        assert_eq!(timer.div(), 0x00);

        // Had the reset cleared only the visible byte, these 56 cycles would
        // carry that hidden 200 past 256 and DIV would already read 1.
        timer.tick(56);
        assert_eq!(timer.div(), 0x00, "the low half of the counter survived the reset");

        timer.tick(200);
        assert_eq!(timer.div(), 0x01);
    }

    #[test]
    fn the_counter_wraps_without_reporting_an_interrupt() {
        let mut timer = Timer::new();

        // A free-running counter rolling over is not an event. The timer
        // interrupt comes from TIMA overflowing, and TAC leaves it disabled.
        for _ in 0..256 {
            assert!(!timer.tick(256), "tick reported an interrupt it has no source for");
        }
        assert_eq!(timer.div(), 0x00, "65536 cycles is exactly one full wrap");
    }

    #[test]
    fn tima_counts_at_the_rate_tac_selects() {
        let mut timer = Timer::new();
        timer.tac = 0b101; // enabled, one tick per 16 cycles

        timer.tick(15);
        assert_eq!(timer.tima, 0, "TIMA moved before a full period elapsed");

        timer.tick(1);
        assert_eq!(timer.tima, 1);

        timer.tick(16 * 4);
        assert_eq!(timer.tima, 5);
    }

    #[test]
    fn the_rate_bits_select_a_rate() {
        let mut timer = Timer::new();
        timer.tac = 0b100; // enabled, slowest rate: one tick per 1024 cycles

        timer.tick(16);
        assert_eq!(timer.tima, 0, "TIMA ran at the 16-cycle rate, not the 1024-cycle one");

        timer.tick(1024 - 16);
        assert_eq!(timer.tima, 1);
    }

    #[test]
    fn tac_bit_2_gates_tima_but_not_div() {
        let mut timer = Timer::new();
        timer.tac = 0b001; // a rate is selected, but the enable bit is clear

        timer.tick(10_000);
        assert_eq!(timer.tima, 0, "TIMA counted with the timer disabled");
        assert_eq!(timer.div(), 39, "DIV runs off the same counter and TAC must not stop it");
    }

    #[test]
    fn overflow_reloads_from_tma_and_reports_the_interrupt() {
        let mut timer = Timer::new();
        timer.tac = 0b101;
        timer.tma = 0xF0;
        timer.tima = 0xFF;

        let fired = timer.tick(16);

        assert!(fired, "TIMA overflowed without requesting the timer interrupt");
        assert_eq!(timer.tima, 0xF0, "TIMA restarted from zero instead of from TMA");
    }

    #[test]
    fn tma_sets_the_interrupt_period() {
        let mut timer = Timer::new();
        timer.tac = 0b101; // one TIMA tick per 16 cycles
        timer.tma = 0xFF;  // reload at 0xFF, so every tick after the first overflows

        // 256 ticks to climb from 0 to the first overflow, then one each.
        let mut fires = 0;
        for _ in 0..(256 + 3) {
            if timer.tick(16) {
                fires += 1;
            }
        }
        assert_eq!(fires, 4, "TMA did not shorten the period after the first overflow");
    }
}