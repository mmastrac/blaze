use std::{cell::Cell, rc::Rc};

use i8051::{CpuView, MemoryMapper, PortMapper, ReadOnlyMemoryMapper};

pub struct Bank {
    pub bank: Rc<Cell<u8>>,
}

impl Default for Bank {
    fn default() -> Self {
        Self {
            bank: Rc::new(Cell::new(0)),
        }
    }
}

impl PortMapper for Bank {
    type WriteValue = ();
    fn interest<C: CpuView>(&self, cpu: &C, addr: u8) -> bool {
        false
    }
    fn pc_extension<C: CpuView>(&self, cpu: &C) -> u16 {
        self.bank.get() as u16
    }
    fn read<C: CpuView>(&self, cpu: &C, addr: u8) -> u8 {
        unimplemented!()
    }
    fn prepare_write<C: CpuView>(&self, cpu: &C, addr: u8, value: u8) -> Self::WriteValue {
        unimplemented!()
    }
    fn write(&mut self, value: Self::WriteValue) {
        unimplemented!()
    }
}

pub struct ROM {
    rom: Vec<u8>,
}

impl ROM {
    pub fn new(rom: Vec<u8>) -> Self {
        Self { rom }
    }
}

impl ReadOnlyMemoryMapper for ROM {
    fn read<C: CpuView>(&self, cpu: &C, addr: u32) -> u8 {
        if addr >= self.rom.len() as u32 {
            return 0xFF;
        }
        self.rom[addr as usize]
    }

    fn len(&self) -> u32 {
        self.rom.len() as u32
    }
}

#[derive(Default)]
pub struct RAM {}

impl MemoryMapper for RAM {
    type WriteValue = u8;
    fn len(&self) -> u32 {
        0x8000
    }
    fn read<C: CpuView>(&self, cpu: &C, addr: u32) -> u8 {
        0xFF
    }
    fn prepare_write<C: CpuView>(&self, cpu: &C, addr: u32, value: u8) -> Self::WriteValue {
        value
    }
    fn write(&mut self, value: Self::WriteValue) {}
}
