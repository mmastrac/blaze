use std::cell::Cell;
use std::rc::Rc;

use i8051::sfr::{SFR_P1, SFR_P2, SFR_P3};
use i8051::{CpuView, MemoryMapper, PortMapper};
use tracing::trace;

#[derive(Default)]
pub struct RAM {}

impl MemoryMapper for RAM {
    type WriteValue = (u32, u8);
    fn len(&self) -> u32 {
        0x8000
    }
    fn read<C: CpuView>(&self, cpu: &C, addr: u32) -> u8 {
        trace!("RAM read {:02X} @ {:X}", addr, cpu.pc_ext());
        if addr == 0x7FFB { 0 } else { 0xFF }
    }
    fn prepare_write<C: CpuView>(&self, cpu: &C, addr: u32, value: u8) -> Self::WriteValue {
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
}

impl Ports {
    pub fn new(rom_bank: Rc<Cell<u8>>) -> Self {
        Self {
            p1: 0,
            p2: 0xff,
            p3: 0xff,
            p3_read: 0b1111_1111,
            rom_bank,
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
                let p1_5 = (value & (1 << 5)) != 0;
                let p1_6 = (value & (1 << 6)) != 0;
                let p1_7 = (value & (1 << 7)) != 0;
                let bank = p1_7 as u8 | ((p1_6 as u8) << 1) | ((p1_5 as u8) << 2);
                self.rom_bank.set(bank);
                self.p1 = value;
            }
            SFR_P2 => self.p2 = value,
            SFR_P3 => self.p3 = value,
            _ => unreachable!(),
        }
    }
}
