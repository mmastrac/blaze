use tracing::trace;

/// Simple emulation of a DEC-style / ER5911 / 93C46-like 3-wire serial NVRAM
/// in 128×8 mode (1 Kbit), but with `tick(...) -> (do, ready)`.
///
/// `ready = true` → device is idle / readable
/// `ready = false` → device is in an internal write/erase cycle (our simulated BUSY)
pub struct Nvr {
    pub mem: [u8; 128],
    pub write_count: usize,

    state: State,
    w_enable: bool,

    last_cs: bool,
    last_sk: bool,

    do_line: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Idle,
    ShiftCmd { bits: u8, shift: u16 },
    ReadOut { addr: u8, bit_pos: u8, data: u8 },
    WriteData { addr: u8, bits: u8, data: u8 },
    Busy { countdown: u8 },
}

impl Default for Nvr {
    fn default() -> Self {
        Self::new()
    }
}

impl Nvr {
    pub fn new() -> Self {
        Self {
            mem: [0; 128],
            state: State::Idle,
            w_enable: false,
            last_cs: false,
            last_sk: false,
            do_line: false,
            write_count: 0,
        }
    }

    /// Tick the NVR with current bus lines and return (DO, READY).
    ///
    /// - `cs`: chip select (active high)
    /// - `sk`: serial clock
    /// - `di`: data in (from MCU)
    ///
    /// Returns:
    /// - `do`: what the chip drives on DO
    /// - `ready`: false when we're in an internal write/erase cycle
    pub fn tick(&mut self, cs: bool, sk: bool, di: bool) -> (bool, bool) {
        // Deselect → reset
        if !cs {
            if self.last_cs {
                trace!("NVR: chip select falling edge");
            }
            self.state = State::Idle;
            self.do_line = false;
            self.last_cs = cs;
            self.last_sk = sk;
            return (self.do_line, true);
        }

        // CS rising edge → start command
        if cs && !self.last_cs {
            trace!("NVR: chip select rising edge");
            self.state = State::ShiftCmd { bits: 0, shift: 0 };
            self.do_line = false;
        }

        // SK rising → sample DI
        if cs && sk && !self.last_sk {
            trace!("NVR: clock tick, DI = {}", di as u8);
            match self.state {
                State::ShiftCmd {
                    mut bits,
                    mut shift,
                } => {
                    shift = (shift << 1) | (di as u16);
                    bits += 1;
                    if bits == 5 + 7 + 1 {
                        self.decode_command(shift);
                    } else {
                        self.state = State::ShiftCmd { bits, shift };
                    }
                }
                State::WriteData {
                    addr,
                    mut bits,
                    mut data,
                } => {
                    data = (data << 1) | (di as u8);
                    bits += 1;
                    if bits == 8 {
                        trace!("NVR: WRITE {addr:02X} = {data:02X}");
                        self.write_count += 1;
                        if self.w_enable {
                            self.mem[addr as usize] = data;
                        }
                        self.state = State::Busy { countdown: 2 };
                        self.do_line = true;
                    } else {
                        self.state = State::WriteData { addr, bits, data };
                    }
                }
                State::ReadOut { .. } | State::Busy { .. } | State::Idle => {}
            }
        }

        // SK falling → advance read / busy
        if cs && !sk && self.last_sk {
            match self.state {
                State::ReadOut {
                    mut addr,
                    mut bit_pos,
                    data,
                } => {
                    let bit = if bit_pos == 0 {
                        false
                    } else {
                        let shift = 8 - bit_pos;
                        ((data >> shift) & 1) != 0
                    };
                    self.do_line = bit;

                    bit_pos += 1;
                    if bit_pos > 8 {
                        addr = addr.wrapping_add(1) & 0x7F;
                        let next = self.mem[addr as usize];
                        self.state = State::ReadOut {
                            addr,
                            bit_pos: 0,
                            data: next,
                        };
                    } else {
                        self.state = State::ReadOut {
                            addr,
                            bit_pos,
                            data,
                        };
                    }
                }
                State::Busy { mut countdown } => {
                    if countdown > 0 {
                        countdown -= 1;
                        if countdown == 0 {
                            self.state = State::Idle;
                            self.do_line = false;
                        } else {
                            self.state = State::Busy { countdown };
                        }
                    }
                }
                _ => {}
            }
        }

        self.last_cs = cs;
        self.last_sk = sk;

        let ready = !matches!(self.state, State::Busy { .. });
        (self.do_line, ready)
    }

    fn decode_command(&mut self, cmd: u16) {
        // 12 bits:
        // S OOOO AAAAAAA
        let start = (cmd >> 11) & 1;
        let op = (cmd >> 7) & 0b1111;
        let addr = (cmd & 0x7F) as u8;

        trace!(
            "NVR: command decoded: {:02X} = {start:01b} {op:04b} {addr:07b}",
            cmd
        );

        if start == 0 {
            self.state = State::Idle;
            return;
        }

        match op {
            0b1000 => {
                trace!("NVR: READ {addr:02X} = {:02X}", self.mem[addr as usize]);
                let data = self.mem[addr as usize];
                self.state = State::ReadOut {
                    addr,
                    bit_pos: 0,
                    data,
                };
                self.do_line = false;
            }
            0b0100 | 0b1100 => {
                trace!("NVR: WRITE {addr:02X}");
                if self.w_enable {
                    self.state = State::WriteData {
                        addr,
                        bits: 0,
                        data: 0,
                    };
                } else {
                    self.state = State::Idle;
                }
            }
            0b0011 => {
                self.w_enable = true;
            }
            0b0010 => {
                self.w_enable = false;
            }
            0b0001 => {
                // ERAL
                if self.w_enable {
                    for b in self.mem.iter_mut() {
                        *b = 0xFF;
                    }
                    self.state = State::Busy { countdown: 2 };
                    self.do_line = true;
                    return;
                }
                self.state = State::Idle;
            }
            _ => {
                self.state = State::Idle;
            }
        }
    }
}
