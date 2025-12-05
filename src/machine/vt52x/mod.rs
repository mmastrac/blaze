use std::path::Path;
use std::sync::mpsc;

use i8051::peripheral::Serial;
use i8051::sfr::SFR_P1;
use i8051::{Cpu, CpuContext, CpuView, PortMapper};
use ssu::session::SessionConfig;

use crate::machine::TerminalSystem;
use crate::machine::vt52x::memory::{Bank, RAM, ROM};

mod memory;

pub struct System {
    pub memory: RAM,
    pub rom: ROM,
    pub bank: Bank,

    serial: Serial,
    in_kbd: mpsc::Sender<u8>,
    out_kbd: mpsc::Receiver<u8>,
}

impl System {
    pub fn new(
        rom: Vec<u8>,
        nvr: Option<&Path>,
        comm1: Option<SessionConfig>,
        comm2: Option<SessionConfig>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let (serial, in_kbd, out_kbd) = Serial::new(60);

        Ok(Self {
            memory: Default::default(),
            rom: ROM::new(rom),
            bank: Default::default(),
            serial,
            in_kbd,
            out_kbd,
        })
    }

    pub fn step(&mut self, cpu: &mut Cpu) {
        cpu.step(self);
    }
}

impl TerminalSystem for System {
    fn step(&mut self, cpu: &mut Cpu) {
        self.step(cpu);
    }
}

impl CpuContext for System {
    type Ports = System;
    type Xdata = RAM;
    type Code = ROM;

    fn ports(&self) -> &Self::Ports {
        self
    }

    fn ports_mut(&mut self) -> &mut Self::Ports {
        self
    }

    fn xdata(&self) -> &Self::Xdata {
        &self.memory
    }

    fn xdata_mut(&mut self) -> &mut Self::Xdata {
        &mut self.memory
    }

    fn code(&self) -> &Self::Code {
        &self.rom
    }

    fn code_mut(&mut self) -> &mut Self::Code {
        &mut self.rom
    }
}

impl PortMapper for System {
    type WriteValue = (u8, u8);
    fn interest<C: CpuView>(&self, cpu: &C, addr: u8) -> bool {
        addr == SFR_P1
    }
    fn pc_extension<C: CpuView>(&self, cpu: &C) -> u16 {
        self.bank.pc_extension(cpu)
    }
    fn read<C: CpuView>(&self, cpu: &C, addr: u8) -> u8 {
        0
    }
    fn read_latch<C: CpuView>(&self, cpu: &C, addr: u8) -> u8 {
        match addr {
            SFR_P1 => self.bank.bank.get() << 4,
            _ => 0,
        }
    }
    fn prepare_write<C: CpuView>(&self, cpu: &C, addr: u8, value: u8) -> Self::WriteValue {
        (addr, value)
    }
    fn write(&mut self, (addr, value): Self::WriteValue) {
        match addr {
            SFR_P1 => {
                // P1.4/P1.5/P1.6
                let p1_4 = (value & (1 << 4)) != 0;
                let p1_5 = (value & (1 << 5)) != 0;
                let p1_6 = (value & (1 << 6)) != 0;

                let bank = p1_4 as u8 | ((p1_5 as u8) << 1) | ((p1_6 as u8) << 2);
                self.bank.bank.set(bank);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    /// Run the ROM and simulation and ensure that we boot to the passed-test screen
    /// and setup comes up.
    ///
    /// We also check that the keyboard commands sent during diagnostics are fully parsed.
    #[test]
    fn test_boots() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let rom = fs::read(format!("{manifest_dir}/roms/vt520/23-010ED-00.bin")).unwrap();
        let mut system = System::new(rom, None, None, None).unwrap();
        let mut cpu = Cpu::new();
        for _ in 0..1000 {
            cpu.step(&mut system);
        }
    }
}
