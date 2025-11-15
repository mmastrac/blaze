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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncPhase {
    VSync(u16),
    Active(u16),
    FrontPorch(u16),
    BackPorch(u16),
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

    pub fn phase(&self) -> SyncPhase {
        let v_sync_start = 0;
        let v_sync_end = v_sync_start + self.t.v_sync;
        let in_vsync = self.y >= v_sync_start && self.y < v_sync_end;

        // vsync -> bp -> active -> fp -> vsync
        if in_vsync {
            SyncPhase::VSync(self.y.saturating_sub(v_sync_start))
        } else {
            let y = self.y.saturating_sub(v_sync_end);
            if y < self.t.v_bp {
                SyncPhase::BackPorch(y)
            } else {
                let y = y.saturating_sub(self.t.v_bp);
                if y < self.t.v_active {
                    SyncPhase::Active(y)
                } else {
                    SyncPhase::FrontPorch(y.saturating_sub(self.t.v_active))
                }
            }
        }
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
