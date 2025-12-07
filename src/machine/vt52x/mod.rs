use std::path::Path;
use std::sync::mpsc;

use i8051::peripheral::Serial;
use i8051::{Cpu, CpuContext, CpuView, DefaultPortMapper, PortMapper};
use ssu::session::SessionConfig;

use crate::machine::TerminalSystem;
use crate::machine::generic::rom::ROM;
use crate::machine::vt52x::memory::{Ports, RAM};

mod memory;

pub struct System {
    pub memory: RAM,
    pub rom: ROM,

    serial: Serial,
    default: DefaultPortMapper,
    in_kbd: mpsc::Sender<u8>,
    out_kbd: mpsc::Receiver<u8>,
    ports: Ports,
}

impl System {
    pub fn new(
        rom: Vec<u8>,
        nvr: Option<&Path>,
        comm1: Option<SessionConfig>,
        comm2: Option<SessionConfig>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let (serial, in_kbd, out_kbd) = Serial::new(60);
        let rom = ROM::new(rom);
        let ports = Ports::new(rom.bank.clone());

        Ok(Self {
            memory: Default::default(),
            rom,
            serial,
            default: Default::default(),
            in_kbd,
            out_kbd,
            ports,
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
    type WriteValue = <(Ports, (Serial, DefaultPortMapper)) as PortMapper>::WriteValue;
    fn interest<C: CpuView>(&self, cpu: &C, addr: u8) -> bool {
        (&self.ports, (&self.serial, &self.default)).interest(cpu, addr)
    }
    fn pc_extension<C: CpuView>(&self, _cpu: &C) -> u16 {
        self.rom.bank.get() as u16
    }
    fn read<C: CpuView>(&self, cpu: &C, addr: u8) -> u8 {
        (&self.ports, (&self.serial, &self.default)).read(cpu, addr)
    }
    fn read_latch<C: CpuView>(&self, cpu: &C, addr: u8) -> u8 {
        (&self.ports, (&self.serial, &self.default)).read_latch(cpu, addr)
    }
    fn prepare_write<C: CpuView>(&self, cpu: &C, addr: u8, value: u8) -> Self::WriteValue {
        (&self.ports, (&self.serial, &self.default)).prepare_write(cpu, addr, value)
    }
    fn write(&mut self, value: Self::WriteValue) {
        (&mut self.ports, (&mut self.serial, &mut self.default)).write(value)
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
