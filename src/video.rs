//! Video timing constants for the VT420 terminal. These are based on some
//! experiments where we ensure that the function that waits for the composite
//! sync signal passes correctly, and the self-test for number of csync pulses
//! per frame returns both the correct timing and correct number of pulses.

pub const TIMING_60HZ: Timing = Timing {
    h_active: 20,
    h_fp: 2,
    h_sync: 6,
    h_bp: 4,       // Htot = 32
    v_active: 417, // Expected by ROM
    v_fp: 4,
    v_sync: 16,
    v_bp: 188, // Vtot = 625
};

pub const TIMING_70HZ: Timing = Timing {
    h_active: 20,
    h_fp: 2,
    h_sync: 6,
    h_bp: 4,       // Htot = 32
    v_active: 417, // Expected by ROM
    v_fp: 3,
    v_sync: 16,
    v_bp: 100, // Vtot = 536
};

#[derive(Clone, Copy, Debug)]
pub struct Timing {
    pub h_active: u16,
    pub h_fp: u16,
    pub h_sync: u16,
    pub h_bp: u16, // h_active + h_fp + h_sync + h_bp = Htot

    pub v_active: u16,
    pub v_fp: u16,
    pub v_sync: u16, // lines of vertical sync (serrated)
    pub v_bp: u16,   // v_active + v_fp + v_sync + v_bp = Vtot
}

impl Timing {
    pub fn htot(&self) -> u16 {
        self.h_active + self.h_fp + self.h_sync + self.h_bp
    }
    pub fn vtot(&self) -> u16 {
        self.v_active + self.v_fp + self.v_sync + self.v_bp
    }
    #[cfg(test)]
    pub fn pixel_tot(&self) -> u16 {
        self.htot() * self.vtot()
    }
}

#[derive(Debug)]
pub struct SyncGen {
    pub t: Timing,
    pub x: u16, // 0..Htot-1
    pub y: u16, // 0..Vtot-1
}

impl SyncGen {
    pub fn new(t: Timing) -> Self {
        Self { t, x: 0, y: 0 }
    }

    /// Advance by one pixel clock. Returns true if CSYNC is set. CSYNC is
    /// active (low) when in HSYNC or VSYNC. This function returns the inverse
    /// of the pin signal.
    pub fn tick(&mut self) -> bool {
        // Compute “in hsync” window for the current line
        let hsync_start = 0;
        let hsync_end = hsync_start + self.t.h_sync;
        let in_hsync = self.x >= hsync_start && self.x < hsync_end;

        // Compute vertical region
        let v_sync_start = 0;
        let v_sync_end = v_sync_start + self.t.v_sync;
        let in_vsync = self.y >= v_sync_start && self.y < v_sync_end;

        // Serration: during vsync, keep producing hsync-rate pulses.
        // CSYNC is active (low) whenever we are in HSYNC OR VSYNC.
        // During VSYNC we *also* go high during the HSYNC window (“serration”).
        let csync = if in_vsync {
            // high during the hsync portion (serrations)
            !in_hsync || (self.y == v_sync_end - 1 && self.x == 2)
        } else {
            // normal: low during the hsync portion
            in_hsync
        };

        // trace!("HSYNC: {} VSYNC: {} CSYNC: {}", in_hsync, in_vsync, csync);

        // Advance raster
        self.x += 1;
        if self.x == self.t.htot() {
            self.x = 0;
            self.y += 1;
            if self.y == self.t.vtot() {
                self.y = 0;
            }
        }

        csync
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_gen_60hz() {
        let mut sync_gen = SyncGen::new(TIMING_60HZ);
        let mut csync_low = false;
        let mut line_count = 0;
        for _ in 0..TIMING_60HZ.pixel_tot() {
            let next = sync_gen.tick();
            if csync_low && !next {
                line_count += 1;
            }
            csync_low = next;
        }

        assert_eq!(sync_gen.x, 0);
        assert_eq!(sync_gen.y, 0);

        assert_eq!(line_count, TIMING_60HZ.vtot());
        assert!((0x4e00..0x4f00).contains(&TIMING_60HZ.pixel_tot()));
    }

    #[test]
    fn test_sync_gen_70hz() {
        let mut sync_gen = SyncGen::new(TIMING_70HZ);
        let mut csync_low = false;
        let mut line_count = 0;
        for _ in 0..TIMING_70HZ.pixel_tot() {
            let next = sync_gen.tick();
            if csync_low && !next {
                line_count += 1;
            }
            csync_low = next;
        }

        assert_eq!(sync_gen.x, 0);
        assert_eq!(sync_gen.y, 0);

        assert_eq!(line_count, TIMING_70HZ.vtot());
        assert!((0x4300..0x4400).contains(&TIMING_70HZ.pixel_tot()));
    }

    #[test]
    fn test_32_clocks_per_line() {
        assert_eq!(TIMING_60HZ.htot(), 32);
        assert_eq!(TIMING_70HZ.htot(), 32);
    }

    #[test]
    fn test_line_count() {
        assert_eq!(TIMING_60HZ.vtot(), 625);
        assert_eq!(TIMING_70HZ.vtot(), 536);
    }

    /// Syncable means that we capture pulses of 15 off, then 15 on at some point.
    #[test]
    fn test_syncable() {
        for timing in [TIMING_60HZ, TIMING_70HZ] {
            let mut runs = Vec::new();
            let mut current_value = None;
            let mut current_count = 0;
            let mut sync_gen = SyncGen::new(timing);
            for _ in 0..timing.pixel_tot() / 4 {
                sync_gen.tick();
            }

            for _ in 0..timing.pixel_tot() {
                let next = sync_gen.tick();
                if Some(next) == current_value {
                    current_count += 1;
                } else {
                    if let Some(value) = current_value {
                        runs.push((value, current_count));
                    }
                    current_value = Some(next);
                    current_count = 1;
                }
            }
            runs.push((current_value.unwrap(), current_count));
            println!("Runs: {:?}", runs);
            assert!(
                runs.windows(2)
                    .any(|w| w[0].0 && w[0].1 >= 15 && !w[1].0 && w[1].1 >= 15)
            );
        }
    }
}
