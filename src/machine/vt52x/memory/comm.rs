use std::collections::VecDeque;

use super::{LOW_XDATA_BASE, RAM, vt52x};

pub struct CommRegs {
    pub mode: u8,
    pub tx: u8,
    pub int: u8,
    pub xoff_level: u8,
    pub buf_page: u8,
    pub buf_mask: u8,
    pub rx_status: u8,
    pub resume_level: u8,
    pub speed: u8,
    pub enable: u8,
    pub status: u8,
}

pub const COMM_REGS: [CommRegs; 3] = [
    CommRegs {
        mode: 0xEB,
        tx: 0xEE,
        int: 0xF0,
        xoff_level: 0xF1,
        buf_page: 0xF2,
        buf_mask: 0xF3,
        rx_status: 0xF5,
        resume_level: 0xD9,
        speed: 0xEC,
        enable: 0xED,
        status: 0xDB,
    },
    CommRegs {
        mode: 0x90,
        tx: 0x93,
        int: 0x95,
        xoff_level: 0x96,
        buf_page: 0x98,
        buf_mask: 0x99,
        rx_status: 0x9B,
        resume_level: 0x97,
        speed: 0x91,
        enable: 0x92,
        status: 0x9C,
    },
    CommRegs {
        mode: 0xE0,
        tx: 0xE3,
        int: 0xE5,
        xoff_level: 0xE6,
        buf_page: 0xE7,
        buf_mask: 0xE8,
        rx_status: 0xEA,
        resume_level: 0xD8,
        speed: 0xE1,
        enable: 0xE2,
        status: 0xDA,
    },
];

const COMM_MODE_LOOPBACK: u8 = 0x20;
const COMM_RX_READY: u8 = 0x40;
const COMM_RX_ACK: u8 = 0x08;
const COMM_RX_OVERFLOW: u8 = 0x10;
const COMM_RX_SEEN: u8 = 0x80;
const COMM_INT_XOFF: u8 = 0x02;
const COMM_INT_RESUME: u8 = 0x04;
const COMM_INT_TX: u8 = 0x08;
const COMM_LEVEL_UNIT: usize = 16;

#[derive(Default)]
pub struct CommChannel {
    pub rx_pending: usize,
    pub rx_overflow: bool,
    pub rx_seen: bool,
    rx_page: u8,
    rx_offset: u8,
    int_enable: u8,
    tx_busy: usize,
    tx_pending: u8,
    pub tx_out: VecDeque<u8>,
}

const VT51X_TX_READY: u8 = 0x08;
const VT51X_TX_EMPTY: u8 = 0x04;

impl RAM {
    pub fn comm_receive(&mut self, ch: usize, byte: u8) {
        let regs = &COMM_REGS[ch];
        let (base, mask) = (
            self.regs.regs[regs.buf_page as usize],
            self.regs.regs[regs.buf_mask as usize],
        );
        let chan = &mut self.comm[ch];
        let addr = (chan.rx_page as usize) << 8 | chan.rx_offset as usize;
        self.dram[LOW_XDATA_BASE + addr] = byte;
        chan.rx_offset = chan.rx_offset.wrapping_add(1);
        if chan.rx_offset == 0 {
            chan.rx_page = ((chan.rx_page.wrapping_add(1)) & !mask).wrapping_add(base);
        }
        chan.rx_pending += 1;
        chan.rx_seen = true;
        if chan.rx_pending >= (!mask as usize + 1) * 0x100 {
            chan.rx_overflow = true;
        }
        self.update_int1();
    }

    #[allow(dead_code)]
    pub(super) fn comm_write(&mut self, reg: u8, value: u8) {
        for ch in 0..3 {
            let regs = &COMM_REGS[ch];
            if reg == regs.buf_page {
                self.comm[ch].rx_page = value;
                self.comm[ch].rx_offset = 1;
                self.comm[ch].rx_pending = 0;
                self.comm[ch].rx_seen = false;
                self.comm[ch].rx_overflow = false;
            } else if reg == regs.tx {
                if self.regs.regs[regs.mode as usize] & COMM_MODE_LOOPBACK != 0 {
                    self.comm_receive(ch, value);
                } else {
                    self.comm[ch].tx_out.push_back(value);
                }
                if self.model.is_vt51x() {
                    self.vt51x_transmit(ch);
                } else {
                    self.comm[ch].tx_busy = vt52x::COMM_TX_BYTE_INSTRUCTIONS;
                }
            } else if reg == regs.int {
                self.comm[ch].int_enable = value;
            } else if reg == regs.rx_status {
                if value & COMM_RX_ACK != 0 {
                    self.comm[ch].rx_pending = self.comm[ch].rx_pending.saturating_sub(1);
                }
                if value & COMM_RX_OVERFLOW != 0 {
                    self.comm[ch].rx_overflow = false;
                }
                if value & COMM_RX_SEEN != 0 {
                    self.comm[ch].rx_seen = false;
                }
            }
        }
        self.update_int1();
    }

    pub fn comm_tick(&mut self) {
        let mut sent = false;
        for ch in 0..3 {
            let chan = &mut self.comm[ch];
            if chan.tx_busy == 0 {
                continue;
            }
            chan.tx_busy -= 1;
            if chan.tx_busy == 0 {
                sent = true;
                if self.model.is_vt51x() {
                    self.vt51x_byte_sent(ch);
                }
            }
        }
        if sent {
            self.update_int1();
        }
    }

    fn comm_int_cause(&self, ch: usize) -> u8 {
        let regs = &COMM_REGS[ch];
        let chan = &self.comm[ch];
        let xoff = self.regs.regs[regs.xoff_level as usize] as usize * COMM_LEVEL_UNIT;
        let resume = self.regs.regs[regs.resume_level as usize] as usize * COMM_LEVEL_UNIT;
        let mut cause = 0;
        let tx_ready = if self.model.is_vt51x() {
            self.vt51x_tx_status(ch) & VT51X_TX_READY != 0
        } else {
            chan.tx_busy == 0
        };
        if tx_ready {
            cause |= COMM_INT_TX;
        }
        if xoff != 0 && chan.rx_pending >= xoff {
            cause |= COMM_INT_XOFF & chan.int_enable;
        }
        if chan.rx_pending <= resume {
            cause |= COMM_INT_RESUME & chan.int_enable;
        }
        cause
    }

    fn update_int1(&mut self) {
        self.int1 = (0..3).any(|ch| self.comm_int_cause(ch) & self.comm[ch].int_enable != 0);
    }

    #[allow(dead_code)]
    pub(super) fn comm_read(&self, r: u8, mut v: u8) -> u8 {
        for (ch, regs) in COMM_REGS.iter().enumerate() {
            if r == regs.rx_status {
                v &= !(COMM_RX_READY | COMM_RX_OVERFLOW | COMM_RX_SEEN);
                if self.comm[ch].rx_pending > 0 {
                    v |= COMM_RX_READY;
                }
                if self.comm[ch].rx_overflow {
                    v |= COMM_RX_OVERFLOW;
                }
                if self.comm[ch].rx_seen {
                    v |= COMM_RX_SEEN;
                }
            }
            if r == regs.int {
                v = self.comm_int_cause(ch);
            }
            if self.model.is_vt51x() && r == regs.status {
                v = v & !(VT51X_TX_READY | VT51X_TX_EMPTY) | self.vt51x_tx_status(ch);
            }
        }
        v
    }

    #[allow(dead_code)]
    fn vt51x_transmit(&mut self, ch: usize) {
        let byte_time = self.vt51x_byte_instructions(ch);
        let chan = &mut self.comm[ch];
        chan.tx_pending = (chan.tx_pending + 1).min(2);
        if chan.tx_busy == 0 {
            chan.tx_busy = byte_time;
        }
    }

    fn vt51x_byte_sent(&mut self, ch: usize) {
        if self.comm[ch].tx_pending == 0 {
            return;
        }
        self.comm[ch].tx_pending -= 1;
        if self.comm[ch].tx_pending > 0 {
            self.comm[ch].tx_busy = self.vt51x_byte_instructions(ch);
        }
    }

    fn vt51x_byte_instructions(&self, ch: usize) -> usize {
        const BAUD: [usize; 11] = [
            300, 600, 1200, 2400, 4800, 9600, 19200, 38400, 57600, 76800, 115200,
        ];
        let code = self.regs.regs[COMM_REGS[ch].speed as usize] & 0x0F;
        10_000_000 / BAUD.get(code as usize).copied().unwrap_or(9600)
    }

    fn vt51x_tx_status(&self, ch: usize) -> u8 {
        let chan = &self.comm[ch];
        if self.regs.regs[COMM_REGS[ch].enable as usize] & 1 == 0 {
            return 0;
        }
        let mut status = 0;
        if chan.tx_pending < 2 {
            status |= VT51X_TX_READY;
        }
        if chan.tx_pending == 0 {
            status |= VT51X_TX_EMPTY;
        }
        status
    }
}
