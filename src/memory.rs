use std::cell::Cell;
use std::cell::RefCell;
use std::fs;
use std::io;
use std::path::Path;
use std::rc::Rc;

use i8051::sfr::SFR_P2;
use i8051::{CpuView, MemoryMapper, PortMapper, ReadOnlyMemoryMapper};
use tracing::{info, trace};

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

// #[derive(Clone, Debug)]
// pub struct VideoRow {
//     inner: Rc<RefCell<VideoRowInner>>,
// }

// #[derive(Debug)]
// struct VideoRowInner {
//     chars: [u8; 65536],
//     p2: u8,
//     p2_dirty: Option<u8>,
// }

// impl VideoRow {
//     pub fn new() -> Self {
//         Self {
//             inner: Rc::new(RefCell::new(VideoRowInner {
//                 chars: [0; 65536],
//                 p2: Default::default(),
//                 p2_dirty: Default::default(),
//             })),
//         }
//     }

//     pub fn read_mem(&self, addr: u16) -> u8 {
//         self.inner.borrow().chars[addr as usize]
//     }

//     pub fn write_mem(&self, addr: u16, value: u8) {
//         self.inner.borrow_mut().chars[addr as usize] = value;
//     }

//     pub fn dump(&self, p2: u8) {
//         let mut s = String::with_capacity(256);
//         let mut b = 0;
//         let c = |c| {
//             if c == 0 { ' ' } else if c < 0x20 || c > 0x7e { '.' } else { char::from(c) }
//         };
//         let row = &self.inner.borrow().chars[p2 as usize * 256..(p2 as usize + 1) * 256];
//         for (i, char) in row.iter().enumerate() {
//             match i % 3 {
//                 0 => s.push(c(*char)),
//                 1 => b = (char & 0xf0) >> 4,
//                 _ => s.push(c(b | ((char & 0xf) << 4))),
//             }
//         }
//         trace!("VIDEO {:02X?}: {s}", p2);
//     }
// }

// impl PortMapper for VideoRow {
//     fn interest<C: CpuView>(&self, cpu: &C, addr: u8) -> bool {
//         addr == SFR_P2
//     }
//     fn read<C: CpuView>(&self, cpu: &C, addr: u8) -> u8 {
//         0
//     }
//     fn read_latch<C: CpuView>(&self, cpu: &C, addr: u8) -> u8 {
//         self.inner.borrow().p2
//     }
//     fn write<C: CpuView>(&mut self, cpu: &C, addr: u8, value: u8) {
//         trace!("Video (P2): {value:02X}");
//         let mut borrow = self.inner.borrow_mut();
//         let mut p2 = value;
//         std::mem::swap(&mut p2, &mut borrow.p2);
//         borrow.p2_dirty = Some(p2);
//     }
//     fn tick<C: CpuView>(&mut self, cpu: &C) {
//         let p2 = std::mem::take(&mut self.inner.borrow_mut().p2_dirty);
//         if let Some(p2) = p2 {
//             self.dump(p2);
//             // self.inner.borrow_mut().chars.fill(0);
//         }
//     }
// }

pub struct Bank {
    pub bank: Rc<Cell<bool>>,
}

impl Default for Bank {
    fn default() -> Self {
        Self {
            bank: Rc::new(Cell::new(false)),
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

#[derive(Default)]
pub struct VideoRow {
    pub p2: u8,
}

impl PortMapper for VideoRow {
    type WriteValue = u8;
    fn interest<C: CpuView>(&self, cpu: &C, addr: u8) -> bool {
        addr == SFR_P2
    }
    fn read<C: CpuView>(&self, cpu: &C, addr: u8) -> u8 {
        self.p2
    }
    fn read_latch<C: CpuView>(&self, cpu: &C, addr: u8) -> u8 {
        self.p2
    }
    fn prepare_write<C: CpuView>(&self, cpu: &C, addr: u8, value: u8) -> Self::WriteValue {
        let mut s = String::with_capacity(256);
        let mut b = 0;
        let c = |c| {
            if c == 0 {
                ' '
            } else if c < 0x20 || c > 0x7e {
                '.'
            } else {
                char::from(c)
            }
        };
        if self.p2 >= 0x80 && self.p2 <= 0xb3 {
            let mut rows = std::iter::repeat_n(String::new(), 8).collect::<Vec<_>>();
            for i in 0..256 {
                for bit in 0..8 {
                    let char = cpu.read_xdata(self.p2 as u16 * 256 + i as u16);
                    let b = (char >> bit) & 1;
                    rows[bit as usize].push(if b > 0 { 'X' } else { ' ' });
                }
            }
            for row in rows {
                trace!("VIDEO {:02X?}: {}", self.p2, row);
            }
        } else {
            for i in 0..256 {
                let char = cpu.read_xdata(self.p2 as u16 * 256 + i as u16);
                match i % 3 {
                    0 => s.push(c(char)),
                    1 => b = (char & 0xf0) >> 4,
                    _ => s.push(c(b | ((char & 0xf) << 4))),
                }
            }
            trace!("VIDEO {:02X?}: {s}", self.p2);
        }

        value
    }
    fn write(&mut self, value: Self::WriteValue) {
        self.p2 = value;
    }
}

pub struct RAM {
    pub sram: [u8; 0x8000],  // 32kB
    pub vram: [u8; 0x20000], // 128kB
    pub mapper: [u8; 16],
    pub peripheral: [u8; 0x100],
    pub rom_bank: Rc<Cell<bool>>,
    pub input_queue: RefCell<Vec<u8>>,
}

impl RAM {
    pub fn new(rom_bank: Rc<Cell<bool>>) -> Self {
        let sram = [0; 0x8000];
        let vram = [0; 0x20000];
        let mut mapper = [0; 16];
        let peripheral = [0; 0x100];
        mapper[3] = 0xff;
        mapper[4] = 0xff;
        mapper[5] = 0xf4;
        Self {
            sram,
            vram,
            mapper,
            peripheral,
            rom_bank,
            input_queue: RefCell::new("x".to_string().into_bytes()),
        }
    }
}

fn swizzle_video_ram(addr: u16, bits: u8) -> u16 {
    if bits & 0x10 == 0 {
        addr
    } else {
        // E O E O -> O E E O
        // 2 3 4 5 -> 3 2 4 5 ?
        if addr < 0x0400 { addr ^ 0x0100 } else { addr }
    }
}

fn calculate_mapper_7ff6(mapper: &[u8; 16], mask: u8) -> u8 {
    const C: [u8; 16] = [
        0x0b, 0x0b, 0x0b, 0x0d, 0x0b, 0x04, 0x0b, 0x0d, 0x03, 0x03, 0x03, 0x0d, 0x03, 0x01, 0x03,
        0x0d,
    ];

    let a = mapper[3];
    let b = mapper[4];

    let c4 = (a & 0b0000_1000) != 0;
    let x = if c4 { b } else { a };

    let c0 = (b & 0b0000_1000) != 0;
    let c1 = (a & 0b0100_0000) != 0;
    let c2 = (x & 0b0000_0010) != 0;
    let c3 = (x & 0b0000_0001) != 0;

    let c_idx = c0 as u8 | ((c1 as u8) << 1) | ((c2 as u8) << 2) | ((c3 as u8) << 3);
    let c = C[c_idx as usize];

    // This isn't totally correct, it seems to require a function of all rows
    let mask_bits = match mask & 0b0000_1111 {
        0b0000 => 0b0000,
        0b0100 => 0b1110,
        0b1000 => 0b1011,
        0b1100 => 0b0001,
        _ => 0b0000,
    };

    trace!(
        "RAM A: {:02X?} {a:08b}, B: {:02X?} {b:08b}, C[{:02X?}] = {:02X?} {c:08b} mask: {:02X?}={mask_bits:08b}",
        a, b, c_idx, c, mask
    );

    return c ^ mask_bits;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryTarget {
    SRAM,
    VRAM,
    Mapper,
    DUART,
    Peripheral,
}

impl RAM {
    fn vram_page(&self) -> u32 {
        self.vram_page_value(self.mapper[3])
    }

    fn vram_page_value(&self, value: u8) -> u32 {
        (value & 0x20 != 0) as u32
    }

    fn target_for_addr(&self, mut addr: u16) -> (MemoryTarget, u32) {
        if (0x7ff0..=0x7fff).contains(&addr) {
            (MemoryTarget::Mapper, (addr & 0x0f) as u32)
        } else if (0x7fe0..=0x7eef).contains(&addr) {
            (MemoryTarget::DUART, (addr & 0x0f) as u32)
        } else if (0x7e00..=0x7eff).contains(&addr) && self.mapper[3] & 0x04 == 0 {
            (MemoryTarget::Peripheral, (addr & 0x0ff) as u32)
        } else if addr < 0x8000 {
            let vram_offset = self.vram_page() * 0x8000;
            if (0x200..0x600).contains(&addr) {
                addr = swizzle_video_ram(addr, self.mapper[3]);
            }
            (MemoryTarget::VRAM, vram_offset + addr as u32)
        } else {
            (MemoryTarget::SRAM, (addr - 0x8000) as u32)
        }
    }
}

impl MemoryMapper for RAM {
    type WriteValue = (MemoryTarget, u32, u32, u32, u8);
    fn len(&self) -> u32 {
        self.sram.len() as u32 + self.vram.len() as u32
    }
    fn read<C: CpuView>(&self, cpu: &C, addr: u32) -> u8 {
        let mut addr = addr as u16;

        let (target, offset) = self.target_for_addr(addr);
        match target {
            MemoryTarget::Mapper => match offset {
                0x6 => calculate_mapper_7ff6(&self.mapper, self.vram[1]),
                x => self.mapper[x as usize],
            },
            MemoryTarget::DUART => {
                trace!("RAM read from {}", READ_2681[(offset as usize)]);

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
                    trace!("RAM read EEPROM ready");
                    return 0b0011_0000;
                }
                if addr == 0x7feb {
                    let next = self.input_queue.borrow_mut().remove(0);
                    trace!("RAM Next: {:?}", next as char);
                    return next;
                }

                return 0;
            }
            MemoryTarget::Peripheral => {
                // peripheral
                return self.peripheral[offset as usize];
            }
            MemoryTarget::VRAM => {
                return self.vram[offset as usize];
            }
            MemoryTarget::SRAM => {
                return self.sram[offset as usize];
            }
        }
    }

    fn prepare_write<C: CpuView>(&self, cpu: &C, addr: u32, value: u8) -> Self::WriteValue {
        let pc = cpu.pc_ext();
        let (target, offset) = self.target_for_addr(addr as u16);
        (target, offset, addr, pc, value)
    }

    fn write(&mut self, (target, offset, addr, pc, value): Self::WriteValue) {
        if pc != 0x15B33 && pc != 0x15BB1 && pc != 0x15BCC {
            trace!(
                "RAM write: 0x{:04X} = 0x{:02X} ({:?}) @ {pc:05X}",
                addr, value, value as char
            );
        }

        match target {
            MemoryTarget::Mapper => {
                info!(
                    "Mapper write: 0x{:04X} = 0x{:02X} -> 0x{:02X} @ {pc:05X}",
                    addr, self.mapper[offset as usize], value
                );
                if offset == 0x5 && self.vram_page() ^ self.vram_page_value(value) != 0 {
                    let old = self.vram_page();
                    let new = self.vram_page_value(value);
                    info!("VIDEO: VRAM page changed: {} -> {}", old, new);
                    if old == 1 && new == 0 {
                        let font = &self.vram[0..];
                        std::fs::write("/tmp/font.bin", font).unwrap();
                    }
                }

                if offset == 0x5 {
                    info!("Memory mapper bank write: {:02X}", value);
                    let bank = (value & 0x4) != 0;
                    if bank != self.rom_bank.get() {
                        info!("RAM write bank changed: {}", bank as u8);
                        self.rom_bank.set(bank);
                    }
                }
                self.mapper[offset as usize] = value;
            }
            MemoryTarget::DUART => {
                trace!("DUART write: 0x{:04X} = 0x{:02X}", addr, value);
            }
            MemoryTarget::Peripheral => {
                trace!("Peripheral write: 0x{:04X} = 0x{:02X}", addr, value);
                self.peripheral[offset as usize] = value;
            }
            MemoryTarget::VRAM => {
                self.vram[offset as usize] = value;
            }
            MemoryTarget::SRAM => {
                self.sram[offset as usize] = value;
            }
        }
    }
}

#[derive(Debug)]
pub struct BankDispatch {
    pub id: u8,
    pub dispatch_addr: u32,
    pub target_addr: u32,
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

    pub fn banks(&self) -> impl Iterator<Item = &[u8]> {
        self.rom.chunks(self.bank_size)
    }

    pub fn find_bank_dispatch(&self) -> Vec<BankDispatch> {
        const BANK_SEARCH_LENGTH: usize = 0x250;
        let banks = self.banks().collect::<Vec<_>>();

        // Search for 74 <a> 02 00 <b>
        // Address from other bank is at 0x100 + (2 * <a>)

        let mut dispatches = Vec::new();

        for (offset, bank, other_offset, other) in [
            (0, banks[0], 0x10000, banks[1]),
            (0x10000, banks[1], 0, banks[0]),
        ] {
            for (dispatch_addr, window) in bank[..BANK_SEARCH_LENGTH].windows(5).enumerate() {
                if window[0] == 0x74 && window[2] == 0x02 && window[3] == 0x00 {
                    let a = window[1];
                    let b = window[4];
                    let target = 0x100 as usize + (2 * a as usize);

                    let hi = other[target + 1];
                    let lo = other[target];
                    let addr = (hi as u16) << 8 | lo as u16;
                    dispatches.push(BankDispatch {
                        id: a,
                        dispatch_addr: dispatch_addr as u32 + offset as u32,
                        target_addr: addr as u32 + other_offset as u32,
                    });
                }
            }
        }

        dispatches
    }

    /// Read a byte from memory
    /// Addresses 0x0000-0xFFFF are mapped to current ROM bank, then RAM
    pub fn read(&self, addr: u16) -> u8 {
        let addr = addr as usize;

        // if addr == 0x595C {
        //     trace!("ROM switch");
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
            trace!("Address out of range: {}", addr);
            0xFF // Address out of range
        }
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

impl ReadOnlyMemoryMapper for ROM {
    fn read<C: CpuView>(&self, cpu: &C, addr: u32) -> u8 {
        if addr >= self.rom_size as u32 {
            return 0xFF;
        }
        self.rom[addr as usize]
    }

    fn len(&self) -> u32 {
        self.rom_size as u32
    }
}
