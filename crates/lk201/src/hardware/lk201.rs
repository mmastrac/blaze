use std::sync::mpsc;

use i8051::peripheral::{Serial, Timer, TimerTick};
use i8051::sfr::{SFR_P0, SFR_P1, SFR_P2, SFR_P3};
use i8051::{Cpu, CpuContext, CpuView, PortMapper, ReadOnlyMemoryMapper};

use crate::Key;

const ROW_COUNT: u8 = 18;
const COL_COUNT: u8 = 8;

static LK201_ROM: &[u8] = include_bytes!("23-004M2-00.BIN");
const LK201_ROM_BOOTED_ADDRESS: u16 = 0x0066;

/// Scancode map for the LK201 keyboard.
const SCANCODE_MAP: [u8; ROW_COUNT as usize * COL_COUNT as usize] = [
    0xD4, 0xD3, 0xD2, 0xD1, 0xD0, 0x60, 0x5A, 0x59, // row 0
    0x48, 0x49, 0x4A, 0x4B, 0x4C, 0x4D, 0x4E, 0xAE, // row 1
    0x52, 0x53, 0x54, 0x55, 0xB2, 0xB1, 0xB0, 0xAF, // row 2
    0xC3, 0xC2, 0xC1, 0xC0, 0xBF, 0x5C, 0xC4, 0xCA, // row 3
    0xC9, 0xC8, 0xC7, 0xC6, 0xBE, 0x5D, 0xC5, 0x56, // row 4
    0xCE, 0xCD, 0xCC, 0xCB, 0x58, 0x5E, 0x57, 0x5F, // row 5
    0xD9, 0xD8, 0xD7, 0xD6, 0x64, 0x61, 0x5B, 0x69, // row 6
    0xDE, 0xDD, 0xDC, 0xDB, 0x65, 0x6A, 0x66, 0x6B, // row 7
    0xE3, 0xE2, 0xE1, 0xE0, 0x68, 0x6C, 0x67, 0x6D, // row 8
    0xE8, 0xE7, 0xE6, 0xE5, 0x70, 0x75, 0x71, 0x76, // row 9
    0xED, 0xEC, 0xEB, 0xEA, 0x73, 0x77, 0x72, 0x78, // row 10
    0xF3, 0xF2, 0xF1, 0xF0, 0xEF, 0x79, 0xBC, 0x74, // row 11
    0xF7, 0xA7, 0xBD, 0xF6, 0x8D, 0x7C, 0xF5, 0x8A, // row 12
    0xAB, 0xFC, 0xFB, 0xFA, 0x8E, 0x7D, 0xF9, 0x8B, // row 13
    0x92, 0x96, 0x99, 0x9D, 0xAA, 0x8C, 0x8F, 0xA1, // row 14
    0x93, 0x97, 0xA8, 0x9A, 0x9E, 0x84, 0xA2, 0x80, // row 15
    0x94, 0x98, 0x9B, 0xA9, 0x9F, 0x85, 0xA3, 0x81, // row 16
    0x95, 0x9C, 0xA0, 0xA4, 0x83, 0x86, 0x82, 0x87, // row 17
];

/// Inverse scancode map for the LK201 keyboard.
const INVERSE_SCANCODE_MAP: [u8; 256] = {
    let mut map = [0xff; 256];
    let mut i = 0;
    while i < SCANCODE_MAP.len() {
        map[SCANCODE_MAP[i] as usize] = i as u8;
        i += 1;
    }
    map
};

/// Pattern of the key matrix (low nibble of P1). Outside of 0-9 means "no
/// selection".
const KEY_MATRIX_PATTERN_LOW: [u8; 10] = [
    // Low nibble (0-9)
    1, 2, 3, 4, 5, 0, 6, 7, 8, 9,
];

/// Pattern of the key matrix (high nibble of P1). Outside of 0-7 means "no
/// selection".
const KEY_MATRIX_PATTERN_HIGH: [u8; 8] = [
    // High nibble (0-7)
    10, 11, 12, 13, 14, 15, 16, 17,
];

pub struct ScanCell {
    row: u8,
    col: u8,
}

impl ScanCell {
    pub const fn new(row: u8, col: u8) -> Option<Self> {
        if row < ROW_COUNT && col < COL_COUNT {
            Some(Self { row, col })
        } else {
            None
        }
    }

    pub const fn from_key(key: Key) -> Option<Self> {
        if let Some(scancode) = key.scancode() {
            Self::from_scancode(scancode)
        } else {
            None
        }
    }

    pub const fn from_scancode(scancode: u8) -> Option<Self> {
        let address = INVERSE_SCANCODE_MAP[scancode as usize];
        let row = address / COL_COUNT;
        let col = address % COL_COUNT;
        Self::new(row, col)
    }

    pub const fn scancode(&self) -> u8 {
        SCANCODE_MAP[self.row as usize * COL_COUNT as usize + self.col as usize]
    }
}

/// A hardware simulator for the LK201 keyboard.
pub struct LK201Hardware {
    cpu: Cpu,
    system: LK201System,
    serial_in: mpsc::Sender<u8>,
    serial_out: mpsc::Receiver<u8>,
}

impl LK201Hardware {
    pub fn new() -> Self {
        let (serial, serial_in, serial_out) = Serial::new(60);
        let mut ports = LK201Ports::default();
        ports.ports[0].0 = 0x00;
        ports.ports[1].0 = 0xFF;
        ports.ports[2].0 = 0xFF;
        ports.ports[3].0 = 0xFF;
        let mut this = Self {
            cpu: Cpu::new(),
            system: LK201System {
                ports,
                serial,
                timer: Timer::default(),
            },
            serial_in,
            serial_out,
        };

        // Advance until the keyboard is (mostly) initialized.
        while this.cpu.pc != LK201_ROM_BOOTED_ADDRESS {
            this.tick();
        }

        this
    }

    pub fn press_key(&mut self, cell: ScanCell) {
        self.system.ports.key_matrix[cell.row as usize] |= 1 << cell.col;
    }

    pub fn release_key(&mut self, cell: ScanCell) {
        self.system.ports.key_matrix[cell.row as usize] &= !(1 << cell.col);
    }

    pub fn release_all_keys(&mut self) {
        self.system.ports.key_matrix.fill(0);
    }

    pub fn tick(&mut self) {
        self.cpu.step(&mut self.system);
        self.system.serial.tick(&mut self.cpu);
        let tick = self.system.timer.prepare_tick(&mut self.cpu, &self.system);
        self.system.timer.tick(&mut self.cpu, tick);
    }
}

#[derive(Default)]
struct LK201Ports {
    ports: [(u8, u8); 4],
    key_matrix: [u8; ROW_COUNT as usize],
}

impl PortMapper for LK201Ports {
    type WriteValue = (u8, u8);
    fn interest<C: CpuView>(&self, cpu: &C, addr: u8) -> bool {
        addr == SFR_P0 || addr == SFR_P1 || addr == SFR_P2 || addr == SFR_P3
    }
    fn read<C: CpuView>(&self, cpu: &C, addr: u8) -> u8 {
        // if addr != SFR_P0 {
        //     eprintln!("{:02X} read @ {:04X}", addr, cpu.pc_ext());
        // }
        match addr {
            SFR_P0 => self.ports[0].0,
            SFR_P1 => self.ports[1].0,
            SFR_P2 => self.ports[2].0,
            SFR_P3 => self.ports[3].0,
            _ => unreachable!(),
        }
    }
    fn prepare_write<C: CpuView>(&self, cpu: &C, addr: u8, value: u8) -> Self::WriteValue {
        // if addr != SFR_P1 {
        //     eprintln!("{:02X} write {:02X} @ {:04X}", addr, value, cpu.pc_ext());
        // }
        (addr, value)
    }
    fn write(&mut self, (addr, value): Self::WriteValue) {
        match addr {
            SFR_P0 => self.ports[0].1 = value,
            SFR_P1 => {
                if let Some(low) = KEY_MATRIX_PATTERN_LOW.get((value & 0xf) as usize) {
                    self.ports[0].0 = !self.key_matrix[*low as usize];
                } else if let Some(high) =
                    KEY_MATRIX_PATTERN_HIGH.get(((value & 0xf0) >> 4) as usize)
                {
                    self.ports[0].0 = !self.key_matrix[*high as usize];
                } else {
                    self.ports[0].0 = 0xFF;
                }
            }
            SFR_P2 => {
                self.ports[2].0 = value;
                self.ports[2].1 = value;
            }
            SFR_P3 => self.ports[3].1 = value,
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

struct LK201System {
    ports: LK201Ports,
    serial: Serial,
    timer: Timer,
}

impl PortMapper for LK201System {
    type WriteValue = <(LK201Ports, (Serial, Timer)) as PortMapper>::WriteValue;
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

impl ReadOnlyMemoryMapper for LK201System {
    fn len(&self) -> u32 {
        LK201_ROM.len() as u32
    }
    fn read<C: CpuView>(&self, cpu: &C, addr: u32) -> u8 {
        if addr >= LK201_ROM.len() as u32 {
            return 0xff;
        }
        LK201_ROM[addr as usize]
    }
}

impl CpuContext for LK201System {
    type Ports = LK201System;
    type Xdata = ();
    type Code = LK201System;

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

    use super::*;
    use crate::{
        ALL_KEYS, AutoRepeatRegister, Division, Key, KeyMode, LK201, LK201Command, Led, Volume,
    };
    use tracing::Level;

    #[test]
    fn test_hardware() {
        tracing_subscriber::fmt().with_max_level(Level::INFO).init();
        let mut hardware = LK201Hardware::new();

        for _ in 0..0x1000 {
            hardware.tick();
            if let Ok(byte) = hardware.serial_out.try_recv() {
                eprintln!("serial out: {:02X}", byte);
            }
        }
        eprintln!("ticked");

        let mut scancode_map = [0; ROW_COUNT as usize * COL_COUNT as usize];
        for row in 0..ROW_COUNT {
            for col in 0..COL_COUNT {
                eprintln!("pressing key: row={}, col={}", row, col);
                hardware.press_key(ScanCell::new(row, col).unwrap());
                for _ in 0..0x2000 {
                    hardware.tick();
                    if let Ok(byte) = hardware.serial_out.try_recv() {
                        let index = row as usize * COL_COUNT as usize + col as usize;
                        if scancode_map[index] == 0 {
                            scancode_map[index] = byte;
                        } else {
                            panic!(
                                "scancode map already has a value for row={}, col={}: {:02X}",
                                row, col, scancode_map[index]
                            );
                        }
                        eprintln!(
                            "serial out (press): {:02X} -> {:?}",
                            byte,
                            Key::from_keycode(byte)
                        );
                    }
                }
                hardware.release_all_keys();
                for _ in 0..0x2000 {
                    hardware.tick();
                    if let Ok(byte) = hardware.serial_out.try_recv() {
                        eprintln!("serial out (release): {:02X}", byte);
                    }
                }
            }
        }
        eprintln!("scancode map: {:02X?}", scancode_map);
        assert_eq!(scancode_map, SCANCODE_MAP);
    }

    #[test]
    fn test_hardware_commands() {
        for i in 0x80..=0xFF {
            let mut hardware = LK201Hardware::new();
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
                    let cmd = LK201Command::try_from(&VecDeque::from_iter([i & !0x80, 0]));
                    eprintln!("{i:02X}: Waiting for more data (alias of {cmd:02X?}");
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
            let mut hardware = LK201Hardware::new();
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

    #[test]
    fn print_keys() {
        eprintln!("all keys: {:02X?}", ALL_KEYS);
        eprintln!("scancode map: {:02X?}", SCANCODE_MAP);
        eprintln!("inverse scancode map: {:02X?}", INVERSE_SCANCODE_MAP);
    }
}
