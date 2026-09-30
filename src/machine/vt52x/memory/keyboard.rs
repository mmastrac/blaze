//! The PS/2 keyboard port on the register page, shared by the VT510, VT520
//! and VT525.

use super::{Registers, Vt5xx};

pub const REG_KBD_TX: u8 = 0xF8;
pub const REG_KBD_RX: u8 = 0xFA;
pub const REG_KBD_STATUS: u8 = 0xFB;

const KBD_RX_READY: u8 = 1 << 1;
const KBD_FRAME: u8 = 1 << 4;
const KBD_LINES: u8 = (1 << 5) | (1 << 0);
const KBD_TX_DONE: u8 = 1 << 6;
/// 0x7FFB bit 5: the keyboard clock line.
const KBD_CLOCK: u8 = 1 << 5;
/// Keyboard clock cycles per byte (start, 8 data, parity, stop) and
/// instructions per half cycle. The VT510 power-up test accepts halves of
/// roughly 6 to 60 instructions.
const KBD_FRAME_CLOCKS: usize = 11;
const KBD_CLOCK_HALF: usize = 15;
/// Instructions the clock stays high after the host releases the line before
/// the keyboard starts a byte, so the host sees the whole byte.
const KBD_CLOCK_DELAY: usize = 100;

/// The keyboard clock line. Keyboard bytes waiting to be clocked out,
/// the queue length last seen, and the position within the byte being clocked
/// (0 = idle).
#[derive(Default)]
pub struct ClockLine {
    frames: usize,
    queued: usize,
    pos: usize,
}

impl ClockLine {
    /// Advance the keyboard clock by one instruction. Each byte the keyboard
    /// queues is clocked out as 11 low/high cycles once the host releases the
    /// line (0x7FF9 bit 6).
    fn tick(&mut self, queued: usize, released: bool) {
        if queued > self.queued {
            self.frames += queued - self.queued;
        }
        self.queued = queued;
        if self.pos > 0 {
            self.pos += 1;
            if self.pos > KBD_CLOCK_DELAY + KBD_FRAME_CLOCKS * 2 * KBD_CLOCK_HALF {
                self.pos = 0;
            }
        } else if self.frames > 0 && released {
            self.frames -= 1;
            self.pos = 1;
        }
    }

    fn low(&self) -> bool {
        self.pos > KBD_CLOCK_DELAY && ((self.pos - KBD_CLOCK_DELAY - 1) / KBD_CLOCK_HALF) % 2 == 0
    }
}

/// A loopback plug on the keyboard port (factory test fixture): the line
/// status bits in 0x7FFB (clock and data) follow the line driven by 0x7FF9
/// bit 6. The VT520 drops it by clearing 0x7F30 bit 6, the VT510 by pulsing
/// 0x7FCF bit 0.
#[derive(Default)]
pub struct FactoryPlug {
    pub fitted: bool,
    line: bool,
}

impl FactoryPlug {
    fn write(&mut self, model: Vt5xx, reg: u8, value: u8) {
        if !self.fitted {
            return;
        }
        match reg {
            0xF9 => self.line = value & 0x40 != 0,
            0xCF if model.is_vt51x() && value & 0x01 != 0 => self.line = false,
            0x30 | 0xB0 if !model.is_vt51x() && value & 0x40 == 0 => self.line = false,
            _ => {}
        }
    }
}

impl Registers {
    pub(super) fn kbd_status(&self) -> u8 {
        let lines = if self.factory_plug.fitted {
            if self.factory_plug.line { KBD_LINES } else { 0 }
        } else if self.kbd_clock.low() {
            KBD_LINES & !KBD_CLOCK
        } else {
            KBD_LINES
        };
        let mut status = KBD_TX_DONE | lines;
        if self.kbd.has_data() {
            status |= KBD_RX_READY;
        }
        if self.frame_pending {
            status |= KBD_FRAME;
        }
        status
    }

    pub(super) fn kbd_write(&mut self, reg: u8, value: u8) {
        self.factory_plug.write(self.model, reg, value);
        match reg {
            REG_KBD_TX => self.kbd.command(value),
            REG_KBD_STATUS => {
                if value & KBD_RX_READY != 0 {
                    if let Some(byte) = self.kbd.pop() {
                        self.kbd_latch = byte;
                    }
                }
                if value & KBD_FRAME != 0 {
                    self.frame_pending = false;
                }
            }
            _ => {}
        }
    }

    /// Advance the keyboard clock by one instruction.
    pub fn kbd_line_tick(&mut self) {
        let released = self.regs[0xF9] & 0x40 != 0;
        self.kbd_clock.tick(self.kbd.queued(), released);
    }

    /// INT0 is asserted (low) while a keyboard byte or the frame interrupt is pending.
    pub fn int0_asserted(&self) -> bool {
        self.kbd.has_data() || self.frame_pending
    }
}
