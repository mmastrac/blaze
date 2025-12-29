use std::sync::mpsc;

use i8051::peripheral::{Serial, Timer, TimerTick};
use i8051::sfr::{SFR_P0, SFR_P1, SFR_P2, SFR_P3};
use i8051::{Cpu, CpuContext, CpuView, PortMapper, ReadOnlyMemoryMapper};

use crate::Key;

static LK401_ROM: &[u8] = include_bytes!("23-LK401.BIN");

/// A hardware simulator for the LK401 keyboard.
pub struct LK401Hardware {
    cpu: Cpu,
    system: LK401System,
    serial_in: mpsc::Sender<u8>,
    serial_out: mpsc::Receiver<u8>,
}

impl LK401Hardware {
    pub fn new() -> Self {
        let (serial, serial_in, serial_out) = Serial::new(60);
        let mut ports = LK401Ports::default();
        ports.ports[0].0 = 0x00;
        ports.ports[1].0 = 0xFF;
        ports.ports[2].0 = 0x00;
        ports.ports[3].0 = 0xFF;
        let mut this = Self {
            cpu: Cpu::new(),
            system: LK401System {
                ports,
                serial,
                timer: Timer::default(),
            },
            serial_in,
            serial_out,
        };

        // Advance until the keyboard is (mostly) initialized.
        while this.cpu.pc != 0x0092 {
            this.tick();
        }

        this
    }

    pub fn tick(&mut self) {
        self.cpu.step(&mut self.system);
        self.system.serial.tick(&mut self.cpu);
        let tick = self.system.timer.prepare_tick(&mut self.cpu, &self.system);
        self.system.timer.tick(&mut self.cpu, tick);
        // if self.system.ports.port2_prev & (1 << 1) != 0 && self.system.ports.ports[2].1 & (1 << 1) != 0 {
        //     if self.system.ports.key_index != 0 {
        //         eprintln!("key index reset through hold was {:02X}", self.system.ports.key_index);
        //     }
        //     self.system.ports.key_index = 0;
        //     self.system.ports.key_pressed = false;
        // }
        self.system.ports.port2_prev = self.system.ports.ports[2].1;
    }
}

struct LK401Ports {
    ports: [(u8, u8); 4],
    key_matrix: [bool; 0x70],
    port2_prev: u8,
    key_index: u8,
    key_pressed: bool,
}

impl Default for LK401Ports {
    fn default() -> Self {
        Self {
            ports: [(0, 0); 4],
            key_matrix: [false; 0x70],
            port2_prev: 0,
            key_index: 0,
            key_pressed: false,
        }
    }
}

impl PortMapper for LK401Ports {
    type WriteValue = (u8, u8);
    fn interest<C: CpuView>(&self, cpu: &C, addr: u8) -> bool {
        addr == SFR_P0 || addr == SFR_P1 || addr == SFR_P2 || addr == SFR_P3 || addr == 0x8E
    }
    fn read<C: CpuView>(&self, cpu: &C, addr: u8) -> u8 {
        if addr != SFR_P2 {
            eprintln!("{:02X} read @ {:04X}", addr, cpu.pc_ext());
        }
        match addr {
            SFR_P0 => self.ports[0].0,
            SFR_P1 => self.ports[1].0,
            SFR_P2 => (self.key_pressed as u8) << 2,
            SFR_P3 => self.ports[3].0,
            0x8E => 0,
            _ => unreachable!(),
        }
    }
    fn prepare_write<C: CpuView>(&self, cpu: &C, addr: u8, value: u8) -> Self::WriteValue {
        if addr != SFR_P2 {
            // eprintln!("{:02X} write {:02X} @ {:04X}", addr, value, cpu.pc_ext());
        } else {
            let delta = value ^ self.ports[2].1;
            if delta & (1 << 0) != 0 {
                // eprintln!("P2.0 changed @ {:04X}", cpu.pc_ext());
            }
            if delta & (1 << 1) != 0 {
                // eprintln!("P2.1 changed @ {:04X}", cpu.pc_ext());
            }
            if delta & (1 << 2) != 0 {
                // eprintln!("P2.2 changed @ {:04X}", cpu.pc_ext());
            }
        }
        (addr, value)
    }
    fn write(&mut self, (addr, value): Self::WriteValue) {
        match addr {
            SFR_P0 => self.ports[0].1 = value,
            SFR_P1 => {}
            SFR_P2 => {
                let delta = value ^ self.ports[2].1;
                if delta & (1 << 0) != 0 && (value & (1 << 0) == 0) {
                    self.key_index = (self.key_index + 1) % (self.key_matrix.len() as u8);
                    if self.key_index == 0 {
                        // eprintln!("key index reset, pressed = {}", self.key_pressed);
                        self.key_pressed = false;
                    }
                    let old_key_pressed = self.key_pressed;
                    self.key_pressed = self.key_matrix[self.key_index as usize];
                    if old_key_pressed != self.key_pressed {
                        // eprintln!("+ key index: {:02X}, pressed = {}", self.key_index, self.key_pressed);
                    }
                }
                if delta & (1 << 1) != 0 && (value & (1 << 1) == 0) {
                    self.key_index = self.key_matrix.len() as u8 - 1;
                    // self.key_pressed = self.key_matrix[self.key_index as usize];
                    // eprintln!("key index: {:02X}, pressed = {}", self.key_index, self.key_pressed);
                }

                self.ports[2].1 = value;
            }
            SFR_P3 => self.ports[3].1 = value,
            0x8E => {}
            _ => unreachable!(),
        }
    }
    fn read_latch<C: CpuView>(&self, cpu: &C, addr: u8) -> u8 {
        match addr {
            SFR_P0 => self.ports[0].1,
            SFR_P1 => self.ports[1].1,
            SFR_P2 => self.ports[2].1,
            SFR_P3 => self.ports[3].1,
            _ => unreachable!(),
        }
    }
}

struct LK401System {
    ports: LK401Ports,
    serial: Serial,
    timer: Timer,
}

impl PortMapper for LK401System {
    type WriteValue = <(LK401Ports, (Serial, Timer)) as PortMapper>::WriteValue;
    fn interest<C: CpuView>(&self, cpu: &C, addr: u8) -> bool {
        self.ports.interest(cpu, addr)
            || self.serial.interest(cpu, addr)
            || self.timer.interest(cpu, addr)
    }
    fn read<C: CpuView>(&self, cpu: &C, addr: u8) -> u8 {
        (&self.ports, (&self.serial, &self.timer)).read(cpu, addr)
    }
    fn prepare_write<C: CpuView>(&self, cpu: &C, addr: u8, value: u8) -> Self::WriteValue {
        (&self.ports, (&self.serial, &self.timer)).prepare_write(cpu, addr, value)
    }
    fn write(&mut self, write: Self::WriteValue) {
        (&mut self.ports, (&mut self.serial, &mut self.timer)).write(write)
    }
    fn read_latch<C: CpuView>(&self, cpu: &C, addr: u8) -> u8 {
        (&self.ports, (&self.serial, &self.timer)).read_latch(cpu, addr)
    }
}

impl ReadOnlyMemoryMapper for LK401System {
    fn len(&self) -> u32 {
        LK401_ROM.len() as u32
    }
    fn read<C: CpuView>(&self, cpu: &C, addr: u32) -> u8 {
        if addr >= LK401_ROM.len() as u32 {
            return 0xff;
        }
        LK401_ROM[addr as usize]
    }
}

impl CpuContext for LK401System {
    type Ports = LK401System;
    type Xdata = ();
    type Code = LK401System;

    fn ports(&self) -> &Self::Ports {
        self
    }
    fn ports_mut(&mut self) -> &mut Self::Ports {
        self
    }
    fn xdata(&self) -> &Self::Xdata {
        unreachable!()
    }
    fn xdata_mut(&mut self) -> &mut Self::Xdata {
        unreachable!()
    }
    fn code(&self) -> &Self::Code {
        &self
    }
    fn code_mut(&mut self) -> &mut Self::Code {
        self
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use crate::LK201Command;
    use crate::{ALL_KEYS, AutoRepeatRegister, Division, Key, KeyMode, LK201, Led, Volume};
    use tracing::Level;

    use super::*;

    #[test]
    fn test_hardware_lk401() {
        tracing_subscriber::fmt().with_max_level(Level::INFO).init();
        let mut hardware = LK401Hardware::new();
        //hardware.system.ports.key_matrix[0] = true;
        eprintln!("ready");
        for _ in 0..0x2000 {
            // eprintln!("ticking {:04X}", hardware.cpu.pc);
            hardware.tick();
            if let Ok(byte) = hardware.serial_out.try_recv() {
                eprintln!("serial out: {:02X}", byte);
            }
        }
        hardware.serial_in.send(0xE9).unwrap();
        for _ in 0..0x2000 {
            // eprintln!("ticking {:04X}", hardware.cpu.pc);
            hardware.tick();
            if let Ok(byte) = hardware.serial_out.try_recv() {
                eprintln!("serial out: {:02X}", byte);
            }
        }
        eprintln!("pressing keys");
        let mut matrix = [0_u8; 0x6F];
        for i in 0..hardware.system.ports.key_matrix.len() {
            eprintln!("pressing key {:02X}", i);
            hardware.system.ports.key_matrix[i] = true;
            for _ in 0..0x2000 {
                // eprintln!("ticking {:04X}", hardware.cpu.pc);
                hardware.tick();
                if let Ok(byte) = hardware.serial_out.try_recv() {
                    eprintln!(" - serial out: {:02X}", byte);
                    matrix[i] = byte;
                }
            }
            hardware.system.ports.key_matrix[i] = false;
            for _ in 0..0x2000 {
                // eprintln!("ticking {:04X}", hardware.cpu.pc);
                hardware.tick();
                if let Ok(byte) = hardware.serial_out.try_recv() {
                    eprintln!(" + serial out: {:02X}", byte);
                }
            }
        }
        eprintln!("matrix: {:02X?}", matrix);

        //[00, 5A, 59, 58, 57, C1, 00, 56, C0, BE, BF, 00, 65, 64, C6, CC, CD, D2, C7, C2, B0, AF, AE, 67, 66, C5, D1, D8, DD, D3, CE, C8, C9, B1, 68, CB, D6, D7, E1, E8, E2, D9, DE, C3, 00, 71, D0, DB, DC, EC, ED, E7, E3, F3, 00, D4, 72, E0, E5, E6, F2, FB, F7, BD, A7, AE, 00, EA, EF, EB, F0, FA, A9, 96, 97, 98, 94, 92, F5, F9, F6, 8D, AA, A8, 99, 9A, 9B, 9C, 95, 73, BC, 8E, 8F, 9D, A3, 9E, 9F, A4, 83, A0, 74, 8A, 8B, 8C, A1, A2, 7C, 7D, 80, 81, 82]
        //[00, 5A, 59, 58, 57, C1, 00, 56, C0, BE, BF, 00, 65, 64, C6, CC, CD, D2, C7, C2, B0, AF, AE, 67, 66, C5, D1, D8, DD, D3, CE, C8, C9, B1, 68, CB, D6, D7, E1, E8, E2, D9, DE, C3, AC, 71, D0, DB, DC, EC, ED, E7, E3, F3, B2, D4, 72, E0, E5, E6, F2, FB, F7, BD, A7, AB, AD, EA, EF, EB, F0, FA, A9, 96, 97, 98, 94, 92, F5, F9, F6, 8D, AA, A8, 99, 9A, 9B, 9C, 95, 73, BC, 8E, 8F, 9D, A3, 9E, 9F, A4, 83, A0, 74, 8A, 8B, 8C, A1, A2, 7C, 7D, 80, 81, 82]
    }

    #[test]
    fn test_hardware_commands() {
        for i in 0x80..=0xFF {
            let mut hardware = LK401Hardware::new();
            for _ in 0..0x1000 {
                hardware.tick();
                _ = hardware.serial_out.try_recv();
            }
            hardware.serial_in.send(i).unwrap();
            let mut response = None;
            for _ in 0..0x1000 {
                hardware.tick();
                if let Ok(byte) = hardware.serial_out.try_recv() {
                    response = Some(byte);
                }
            }

            match response {
                None => {
                    eprintln!("{i:02X}: Waiting for more data");
                    hardware.serial_in.send(0x80).unwrap();
                    for _ in 0..0x1000 {
                        hardware.tick();
                        if let Ok(byte) = hardware.serial_out.try_recv() {
                            eprintln!("{byte:02X}");
                        }
                    }
                }
                Some(0xB6) => {}
                Some(0xB7) => {
                    let cmd = LK201Command::try_from(&VecDeque::from_iter([i]));
                    eprintln!("{i:02X}: Keyboard locked (cmd = {cmd:?}");
                }
                Some(0xBA) => {
                    let cmd = LK201Command::try_from(&VecDeque::from_iter([i]));
                    eprintln!("{i:02X}: Mode change ack (cmd = {cmd:?}");
                }
                Some(b) => {
                    eprintln!("{i:02X}: Unknown response ({b:02X})");
                }
            }
        }
    }

    #[test]
    fn test_response() {
        use LK201Command::*;
        for cmd in [
            KeyClickEnable(Volume::new(2).unwrap()),
            LedEnable(Led::new(0x1)),
            EnableRepeat,
            DisableRepeat,
            RepeatToDown,
            TempNoRepeat,
            SoundClick,
            SetAutoRepeat {
                register: AutoRepeatRegister::new(0).unwrap(),
                timeout: 100,
                rate: 30,
            },
            SetMode {
                mode: KeyMode::AutoDown,
                division: Division::new(1).unwrap(),
            },
            SetModeWithAutoRepeat {
                mode: KeyMode::AutoDown,
                division: Division::new(1).unwrap(),
                register: AutoRepeatRegister::new(0).unwrap(),
            },
            SetDefaults,
        ] {
            let mut hardware = LK401Hardware::new();
            for _ in 0..0x1000 {
                hardware.tick();
                _ = hardware.serial_out.try_recv();
            }
            let bytes: Vec<u8> = cmd.into();
            for b in &bytes {
                hardware.serial_in.send(*b).unwrap();
            }
            let mut hw_response = vec![];
            for _ in 0..0x1000 {
                hardware.tick();
                if let Ok(byte) = hardware.serial_out.try_recv() {
                    hw_response.push(byte);
                }
            }
            eprintln!("{cmd:?} ({bytes:02X?}) -> {hw_response:02X?}");
            if let Some(response) = cmd.response() {
                assert_eq!(response.to_bytes(), hw_response);
            } else {
                assert_eq!(hw_response, vec![]);
            }
        }
    }
}
