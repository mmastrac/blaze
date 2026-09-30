use std::cell::Cell;
use std::rc::Rc;

use i8051::sfr::{SFR_P1, SFR_P2, SFR_P3};
use i8051::{CpuView, MemoryMapper, PortMapper};
use tracing::trace;

mod comm;
pub(crate) mod vt51x;
pub(crate) mod vt52x;

use comm::CommChannel;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Vt5xx {
    Vt510,
    #[default]
    Vt520,
    Vt525,
}

impl Vt5xx {
    pub fn is_vt51x(self) -> bool {
        self == Vt5xx::Vt510
    }
}

const PAGE_SIZE: usize = 0x8000;
const PAGE_COUNT: usize = 64;
pub(crate) const DRAM_SIZE: usize = PAGE_SIZE * PAGE_COUNT;
pub const LOW_XDATA_BASE: usize = 4 * PAGE_SIZE;

pub const REG_BLIT_CMD: u8 = 0xC9;

pub struct Registers {
    pub regs: [u8; 256],
    pub blit_count: usize,
    pub unknown_blits: std::collections::BTreeMap<u8, usize>,
    pub blit_busy: usize,
}

impl Default for Registers {
    fn default() -> Self {
        Self {
            regs: [0; 256],
            blit_count: 0,
            unknown_blits: Default::default(),
            blit_busy: 0,
        }
    }
}

impl Registers {
    pub(crate) fn write(&mut self, reg: u8, value: u8, pc: u32) {
        self.regs[reg as usize] = value;
        self.blit_reg_write(reg, value, pc);
    }
}

pub const FB_STRIDE: usize = 0x100;
pub const FB_LINES: usize = 0x200;
const MIN_ROW_LINES: usize = 8;
pub const TEXT_ROWS: usize = FB_LINES / MIN_ROW_LINES;
pub const SCREEN_CELLS: usize = TEXT_ROWS * TEXT_COLS;
pub const TEXT_COLS: usize = FB_STRIDE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextCell {
    pub ch: u8,
    pub attr: u8,
    pub continuation: bool,
    pub soft_glyph: Option<u32>,
    pub rendition: u8,
    pub underline_line: u8,
    pub colour: u8,
    pub font_column: u8,
}

impl Default for TextCell {
    fn default() -> Self {
        Self {
            ch: b' ',
            attr: 0,
            continuation: false,
            soft_glyph: None,
            rendition: 0,
            underline_line: 0,
            colour: 0x70,
            font_column: 0,
        }
    }
}

pub struct RAM {
    pub regs: Registers,
    pub comm: [CommChannel; 3],
    pub dram: Vec<u8>,
    pub text: Vec<TextCell>,
    pub model: Vt5xx,
    pub int1: bool,
}

impl Default for RAM {
    fn default() -> Self {
        Self {
            regs: Registers::default(),
            comm: Default::default(),
            dram: vec![0; DRAM_SIZE],
            text: vec![TextCell::default(); SCREEN_CELLS],
            model: Vt5xx::default(),
            int1: false,
        }
    }
}

impl RAM {
    pub(crate) fn window_format(&self, lower: bool) -> u8 {
        if self.model.is_vt51x() {
            vt51x::window_format(self)
        } else {
            vt52x::window_format(self, lower)
        }
    }

    pub(crate) fn row_lines(&self, lower: bool) -> usize {
        ((self.window_format(lower) & 0x0F) as usize + 1).max(MIN_ROW_LINES)
    }
}

impl MemoryMapper for RAM {
    type WriteValue = (u32, u8);
    fn len(&self) -> u32 {
        0x8000
    }
    fn read<C: CpuView>(&self, cpu: &C, addr: u32) -> u8 {
        trace!("RAM read {:02X} @ {:X}", addr, cpu.pc_ext());
        if addr == 0x7FFB && cpu.pc_ext() != 0x71BA7 {
            0
        } else {
            0xFF
        }
    }
    fn prepare_write<C: CpuView>(&self, _cpu: &C, addr: u32, value: u8) -> Self::WriteValue {
        (addr, value)
    }
    fn write(&mut self, value: Self::WriteValue) {
        trace!("RAM write {:02X} @ {:X}", value.1, value.0);
    }
}

pub struct Ports {
    pub p1: u8,
    pub p2: u8,
    pub p3: u8,
    pub p3_read: u8,
    pub rom_bank: Rc<Cell<u8>>,
    pub model: Vt5xx,
}

impl Ports {
    pub fn new(rom_bank: Rc<Cell<u8>>) -> Self {
        Self {
            p1: 0,
            p2: 0xff,
            p3: 0xff,
            p3_read: 0b1111_1111,
            rom_bank,
            model: Vt5xx::Vt520,
        }
    }

    pub fn tick(&mut self) {}
}

impl PortMapper for Ports {
    type WriteValue = (u8, u8);
    fn interest<C: CpuView>(&self, _cpu: &C, addr: u8) -> bool {
        addr == SFR_P2 || addr == SFR_P3 || addr == SFR_P1
    }
    fn read<C: CpuView>(&self, _cpu: &C, addr: u8) -> u8 {
        match addr {
            SFR_P1 => self.p1,
            SFR_P2 => self.p2,
            SFR_P3 => self.p3_read,
            _ => unreachable!(),
        }
    }
    fn read_latch<C: CpuView>(&self, _cpu: &C, addr: u8) -> u8 {
        match addr {
            SFR_P1 => self.p1,
            SFR_P2 => self.p2,
            SFR_P3 => self.p3,
            _ => unreachable!(),
        }
    }
    fn prepare_write<C: CpuView>(&self, cpu: &C, addr: u8, value: u8) -> Self::WriteValue {
        if addr == SFR_P3 {
            trace!("P3 write {:02X} @ {:X}", value, cpu.pc_ext());
        }
        if addr == SFR_P2 {
            trace!("P2 write {:02X} @ {:X}", value, cpu.pc_ext());
        }
        if addr == SFR_P1 {
            trace!("P1 write {:02X} @ {:X}", value, cpu.pc_ext());
        }
        (addr, value)
    }
    fn write(&mut self, (addr, value): Self::WriteValue) {
        match addr {
            SFR_P1 => {
                let bank = match self.model {
                    Vt5xx::Vt510 => vt51x::rom_bank(value),
                    Vt5xx::Vt520 | Vt5xx::Vt525 => vt52x::rom_bank(value),
                };
                self.rom_bank.set(bank);
                self.p1 = value;
            }
            SFR_P2 => self.p2 = value,
            SFR_P3 => self.p3 = value,
            _ => unreachable!(),
        }
    }
}
