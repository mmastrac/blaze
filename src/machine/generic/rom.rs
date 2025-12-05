use std::cell::Cell;
use std::rc::Rc;

use i8051::{CpuView, ReadOnlyMemoryMapper};

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

pub struct ROM {
    rom: Vec<u8>,
    pub bank: Rc<Cell<u8>>,
}

impl ROM {
    pub fn new(rom: Vec<u8>) -> Self {
        Self {
            rom,
            bank: Rc::new(Cell::new(0)),
        }
    }

    pub fn banks(&self) -> impl Iterator<Item = &[u8]> {
        self.rom.chunks(0x10000)
    }
}

impl ReadOnlyMemoryMapper for ROM {
    fn read<C: CpuView>(&self, _cpu: &C, addr: u32) -> u8 {
        if addr >= self.rom.len() as u32 {
            return 0xFF;
        }
        self.rom[addr as usize]
    }

    fn len(&self) -> u32 {
        self.rom.len() as u32
    }
}
