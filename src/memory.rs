use std::cell::Cell;
use std::cell::RefCell;
use std::fs;
use std::io;
use std::path::Path;
use std::rc::Rc;

use i8051::MemoryMapper;
use i8051::PortMapper;
use i8051::sfr::SFR_P2;

const READ_2681: &[&str] = &[
    "Mode Register A (MR1A, MR2A)",
    "Status Register A (SRA)",
    "BRG Extend",
    "Rx Holding Register A (RHRA)",
    "Input Port Change Register (IPCR)",
    "Interrupt Status Register (ISR)",
    "Counter/Timer Upper Value (CTU)",
    "Counter/Timer Lower Value (CTL)",
    "Mode Register B (MR1B, MR2B)",
    "Status Register B (SRB)",
    "1×/16× Test",
    "Rx Holding Register B (RHRB)",
    "Use for scratch pad",
    "Input Ports IP0 to IP6",
    "Start Counter Command",
    "Stop Counter Command",
];

const WRITE_2681: &[&str] = &[
    "Mode Register A (MR1A, MR2A)",
    "Clock Select Register A (CSRA)",
    "Command Register A (CRA)",
    "Tx Holding Register A (THRA)",
    "Aux. Control Register (ACR)",
    "Interrupt Mask Register (IMR)",
    "C/T Upper Preset Value (CRUR)",
    "C/T Lower Preset Value (CTLR)",
    "Mode Register B (MR1B, MR2B)",
    "Clock Select Register B (CSRB)",
    "Command Register B (CRB)",
    "Tx Holding Register B (THRB)",
    "Use for scratch pad",
    "Output Port Conf. Register (OPCR)",
    "Set Output Port Bits Command",
    "Reset Output Port Bits Command",
];

#[derive(Clone, Debug)]
pub struct VideoRow {
    inner: Rc<RefCell<VideoRowInner>>,
}

#[derive(Debug)]
struct VideoRowInner {
    chars: [u8; 256],
    p2: u8,
    p2_dirty: bool,
}

impl VideoRow {
    pub fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(VideoRowInner {
                chars: [0; 256],
                p2: Default::default(),
                p2_dirty: Default::default(),
            })),
        }
    }

    pub fn dump(&self) {
        let mut s = String::with_capacity(256);
        let mut b = 0;
        let c = |c| {
            if c == 0 { ' ' } else { char::from(c) }
        };
        for (i, char) in self.inner.borrow().chars.iter().enumerate() {
            match i % 3 {
                0 => s.push(c(*char)),
                1 => b = (char & 0xf0) >> 4,
                _ => s.push(c(b | ((char & 0xf) << 4))),
            }
        }
        println!("VIDEO: {s:?}");
    }
}

impl PortMapper for VideoRow {
    fn interest(&self, addr: u8) -> bool {
        addr == SFR_P2
    }
    fn read(&mut self, addr: u8) -> u8 {
        0
    }
    fn read_latch(&mut self, addr: u8) -> u8 {
        self.inner.borrow().p2
    }
    fn write(&mut self, addr: u8, value: u8) {
        println!("Video (P2): {value:02X}");
        self.inner.borrow_mut().p2 = value;
        self.inner.borrow_mut().p2_dirty = true;
    }
    fn tick(&mut self) {
        if std::mem::take(&mut self.inner.borrow_mut().p2_dirty) {
            self.dump();
            self.inner.borrow_mut().chars.fill(0);
        }
    }
}

pub struct RAM {
    pub ram: [u8; 0x10000],
    pub rom_bank: Rc<Cell<bool>>,
    pub input_queue: RefCell<Vec<u8>>,
    pub video_row: VideoRow,
}

impl RAM {
    pub fn new(rom_bank: Rc<Cell<bool>>, video_row: VideoRow) -> Self {
        let mut ram = [0; 0x10000];
        ram[0x7ff3] = 0xff;
        ram[0x7ff4] = 0xff;
        ram[0x7ff5] = 0x04;
        Self {
            ram,
            rom_bank,
            input_queue: RefCell::new("x".to_string().into_bytes()),
            video_row,
        }
    }
}

fn swizzle_video_ram(addr: u16, bits: u8) -> u16 {
    if bits & 0x10 == 0 {
        addr
    } else {
        // E O E O -> O E E O
        // 1 2 3 4 -> 2 1 3 4 ?
        if addr < 0x0400 { addr ^ 0x0100 } else { addr }
    }
}

impl MemoryMapper for RAM {
    fn read(&self, mut addr: u16) -> u8 {
        if addr < 0x100 {
            return self.video_row.inner.borrow_mut().chars[addr as usize];
        }
        if addr & 0xfff0 == 0x7fe0 {
            println!("RAM read from {}", READ_2681[(addr & 0x0f) as usize]);
        }
        if (0x0200..0x0600).contains(&addr) {
            addr = swizzle_video_ram(addr, self.ram[0x7ff3]);
        }
        println!("RAM read: 0x{:04X} = {:02X}", addr, self.ram[addr as usize]);
        if addr == 0x7ff6 {
            const C: [u8; 32] = [
                0x0b, 0x0b, 0x0b, 0x0d, 0x0b, 0x04, 0x0b, 0x0d, 0x03, 0x03, 0x03, 0x0d, 0x03, 0x01,
                0x03, 0x0d, 0x0b, 0x0b, 0x0b, 0x0d, 0x0b, 0x04, 0x0b, 0x0d, 0x03, 0x03, 0x03, 0x0d,
                0x03, 0x01, 0x03, 0x0d,
            ];

            let a = self.ram[0x7ff3];
            let b = self.ram[0x7ff4];

            let c4 = (a & 0b0000_1000) != 0;
            let x = if c4 { b } else { a };

            let c0 = (b & 0b0000_1000) != 0;
            let c1 = (a & 0b0100_0000) != 0;
            let c2 = (x & 0b0000_0010) != 0;
            let c3 = (x & 0b0000_0001) != 0;

            let c_idx = c0 as u8
                | ((c1 as u8) << 1)
                | ((c2 as u8) << 2)
                | ((c3 as u8) << 3)
                | ((c4 as u8) << 4);
            let c = C[c_idx as usize];
            let mask = self.video_row.inner.borrow().chars[1];
            let mask_bits = match mask & 0b0000_1111 {
                0b0000 => 0b0000,
                0b0100 => 0b1110,
                0b1000 => 0b1011,
                0b1100 => 0b0001,
                _ => 0b0000,
            };

            println!(
                "RAM A: {:02X?} {a:08b}, B: {:02X?} {b:08b}, C[{:02X?}] = {:02X?} {c:08b} mask: {:02X?}={mask_bits:08b}",
                a, b, c_idx, c, mask
            );

            return c ^ mask_bits;
        }
        if addr == 0x7fe5 {
            if self.input_queue.borrow().len() > 0 {
                return 0b1111_1101;
            } else {
                return 0b0001_1001;
            }
        }
        if addr == 0x7fe1 {
            return 0xff;
        }
        if addr == 0x7fe3 {
            return 0xff;
        }
        if addr == 0x7fe9 {
            return 0b0000_0000;
        }
        if addr == 0x7fed {
            // eeprom ready
            return 0b0001_0000;
        }
        if addr == 0x8000 {
            // return 0;
        }
        if addr == 0x7feb {
            let next = self.input_queue.borrow_mut().remove(0);
            println!("RAM Next: {:?}", next as char);
            return next;
        }
        self.ram[addr as usize]
    }

    fn write(&mut self, mut addr: u16, value: u8) {
        if addr < 0x100 {
            println!(
                "RAM (video) write: 0x{:04X} = 0x{:02X} ({:?})",
                addr, value, value as char
            );
            self.video_row.inner.borrow_mut().chars[addr as usize] = value;
            return;
        }
        // if addr > 0xff {
        println!(
            "RAM write: 0x{:04X} = 0x{:02X} ({:?})",
            addr, value, value as char
        );
        // }
        if (0x0200..0x0600).contains(&addr) {
            addr = swizzle_video_ram(addr, self.ram[0x7ff3]);
        }

        if addr & 0xfff0 == 0x7fe0 {
            println!("RAM write to {}", WRITE_2681[(addr & 0x0f) as usize]);
        }
        self.ram[addr as usize] = value;
        if addr == 0x7ff5 {
            let bank = (value & 0x4) != 0;
            if bank != self.rom_bank.get() {
                println!("RAM write bank changed: {}", bank as u8);
                self.rom_bank.set(bank);
            }
        }
        if addr == 0x7FE2 || addr == 0x7fea {
            println!("RAM write command {:b}", value >> 4 & 0b111);
        }
    }
}

/// Memory mapper for the VT420 emulator
/// Handles RAM and banked ROM memory regions
pub struct ROM {
    /// ROM data (loaded into memory)
    rom: Vec<u8>,
    /// ROM size in bytes
    rom_size: usize,
    /// Bank size (64KB per bank)
    bank_size: usize,
    rom_bank: Rc<Cell<bool>>,
}

impl ROM {
    /// Create a new memory mapper with ROM loaded from file
    /// Bank 0: first 64KB of ROM, Bank 1: remaining ROM data
    /// Initializes with bank 0 mapped
    pub fn new(rom_path: &Path, rom_bank: Rc<Cell<bool>>) -> io::Result<Self> {
        let rom = fs::read(rom_path)?;
        let rom_size = rom.len();
        let bank_size = 0x10000; // 64KB per bank

        Ok(Self {
            rom,
            rom_size,
            rom_bank,
            bank_size,
        })
    }

    /// Read a byte from memory
    /// Addresses 0x0000-0xFFFF are mapped to current ROM bank, then RAM
    pub fn read(&self, addr: u16) -> u8 {
        let addr = addr as usize;

        // if addr == 0x595C {
        //     println!("ROM switch");
        //     self.rom_bank.set(false);
        // }

        // Check if address is within current ROM bank range
        if addr < self.bank_size {
            let rom_addr = (self.rom_bank.get() as usize * self.bank_size) + addr;
            if rom_addr < self.rom_size {
                self.rom[rom_addr]
            } else {
                0xFF // Bank address out of ROM range
            }
        } else {
            println!("Address out of range: {}", addr);
            0xFF // Address out of range
        }
    }

    /// Write a byte to memory
    pub fn write(&mut self, addr: u16, value: u8) {
        // Writes to ROM are ignored (read-only)
    }

    /// Get the size of the ROM in bytes
    pub fn rom_size(&self) -> usize {
        self.rom_size
    }

    /// Get the current ROM bank (0 = lower half, 1 = upper half)
    pub fn rom_bank(&self) -> u8 {
        self.rom_bank.get() as u8
    }

    /// Set the current ROM bank (0 = lower half, 1 = upper half)
    pub fn set_rom_bank(&mut self, bank: u8) {
        if bank <= 1 {
            self.rom_bank.set(bank == 1);
        }
    }

    /// Get the size of each ROM bank in bytes
    pub fn bank_size(&self) -> usize {
        self.bank_size
    }

    /// Get the number of ROM banks
    pub fn num_banks(&self) -> usize {
        (self.rom_size + self.bank_size - 1) / self.bank_size
    }

    /// Switch to the first 64KB of ROM (bank 0)
    pub fn switch_to_lower_bank(&mut self) {
        self.rom_bank.set(false);
    }

    /// Switch to the remaining ROM data (bank 1)
    pub fn switch_to_upper_bank(&mut self) {
        self.rom_bank.set(true);
    }
}

impl MemoryMapper for ROM {
    fn read(&self, addr: u16) -> u8 {
        self.read(addr)
    }
    fn write(&mut self, addr: u16, value: u8) {
        self.write(addr, value);
    }
}
