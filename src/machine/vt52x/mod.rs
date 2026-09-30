use std::path::Path;
use std::sync::mpsc;

use i8051::peripheral::Serial;
use i8051::{Cpu, CpuContext, CpuView, DefaultPortMapper, PortMapper};
use ssu::session::SessionPartsUnsend;

use crate::machine::TerminalSystem;
use crate::machine::generic::display::{Display, TextAttr, TextLine};
use crate::machine::generic::rom::ROM;
use crate::machine::generic::script::{Script, ScriptHost};
use crate::machine::vt52x::memory::{Ports, RAM, Vt5xx};

pub mod memory;

const FRAME_INSTRUCTIONS: usize = 14_000;

fn rom_model(rom: &[u8]) -> Vt5xx {
    if rom.windows(12).any(|w| w == b"\x05VT525\x05VT100") {
        Vt5xx::Vt525
    } else if rom.windows(6).any(|w| w == b"DEC510") {
        Vt5xx::Vt510
    } else {
        Vt5xx::Vt520
    }
}

pub struct System {
    pub memory: RAM,
    pub rom: ROM,
    pub script: Script,
    pub instruction_count: usize,

    serial: Serial,
    default: DefaultPortMapper,
    in_kbd: mpsc::Sender<u8>,
    ports: Ports,
}

impl System {
    pub fn new(
        rom: Vec<u8>,
        _nvr: Option<&Path>,
        _comm1: Option<SessionPartsUnsend>,
        _comm2: Option<SessionPartsUnsend>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let (serial, in_kbd, _out_kbd) = Serial::new(60);
        let model = rom_model(&rom);
        let rom = ROM::new(rom);
        let mut ports = Ports::new(rom.bank.clone());
        ports.model = model;

        Ok(Self {
            memory: Default::default(),
            rom,
            script: Script::default(),
            instruction_count: 0,
            serial,
            default: Default::default(),
            in_kbd,
            ports,
        })
    }

    pub fn step(&mut self, cpu: &mut Cpu) {
        self.instruction_count += 1;
        if !self.script.is_done() {
            let pc = cpu.pc_ext(self);
            let mut script = std::mem::take(&mut self.script);
            script.run(pc, self);
            self.script = script;
        }
        cpu.step(self);
    }
}

impl TerminalSystem for System {
    fn step(&mut self, cpu: &mut Cpu) {
        self.step(cpu);
    }

    fn exit_code(&self) -> Option<i32> {
        self.script.exit_code()
    }
}

impl ScriptHost for System {
    fn script_keyboard(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            _ = self.in_kbd.send(byte);
        }
    }

    fn script_instructions(&self) -> usize {
        self.instruction_count
    }

    fn script_ticks(&self) -> usize {
        self.instruction_count / FRAME_INSTRUCTIONS
    }
}

impl Display for System {
    fn render_framebuffer(&self, frame: &mut [u8]) {
        frame.fill(0);
    }

    fn render_textbuffer(
        &self,
        _line: &mut dyn FnMut(usize, TextLine),
        _cell: &mut dyn FnMut(usize, usize, char, TextAttr),
    ) {
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
