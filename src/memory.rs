use std::cell::Cell;
use std::cell::RefCell;
use std::fs;
use std::io;
use std::path::Path;
use std::rc::Rc;

use hex_literal::hex;
use i8051::sfr::SFR_P2;
use i8051::sfr::SFR_P3;
use i8051::{CpuView, MemoryMapper, PortMapper, ReadOnlyMemoryMapper};
use tracing::{info, trace};

use crate::nvr::Nvr;
use crate::video::Mapper;
use crate::video::SyncGen;
use crate::video::TIMING_60HZ;
use crate::video::TIMING_70HZ;

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

#[derive(Clone, Debug)]
pub struct SyncHolder {
    pub hz_70: Rc<Cell<bool>>,
    pub sync_gen: Rc<RefCell<SyncGen>>,
}

impl SyncHolder {
    pub fn set_hz_70(&self, value: bool) {
        if self.hz_70.replace(value) != value {
            *self.sync_gen.borrow_mut() =
                SyncGen::new(if value { TIMING_70HZ } else { TIMING_60HZ });
        }
    }
}

impl Default for SyncHolder {
    fn default() -> Self {
        Self {
            hz_70: Rc::new(Cell::new(false)),
            sync_gen: Rc::new(RefCell::new(SyncGen::new(TIMING_60HZ))),
        }
    }
}

pub struct VideoProcessor {
    pub p2: u8,
    pub p3: u8,
    pub p3_read: u8,
    pub sync: SyncHolder,
}

impl VideoProcessor {
    pub fn new() -> Self {
        Self {
            p2: 0xff,
            p3: 0xff,
            p3_read: 0b1111_1111,
            sync: SyncHolder::default(),
        }
    }

    pub fn tick(&mut self) {
        // Set the T0 bit (bit 4)
        let csync_low = self.sync.sync_gen.borrow_mut().tick();
        self.p3_read &= !(1 << 4);
        self.p3_read |= (csync_low as u8) << 4;
    }
}

impl PortMapper for VideoProcessor {
    type WriteValue = (u8, u8);
    fn interest<C: CpuView>(&self, cpu: &C, addr: u8) -> bool {
        addr == SFR_P2 || addr == SFR_P3
    }
    fn read<C: CpuView>(&self, cpu: &C, addr: u8) -> u8 {
        if addr == SFR_P3 {
            // trace!("P3 read {:02X} @ {:X}", self.p3_read, cpu.pc_ext());
        }
        match addr {
            SFR_P2 => self.p2,
            SFR_P3 => self.p3_read,
            _ => unreachable!(),
        }
    }
    fn read_latch<C: CpuView>(&self, cpu: &C, addr: u8) -> u8 {
        if addr == SFR_P3 {
            // trace!("P3 read latch {:02X} @ {:X}", self.p3, cpu.pc_ext());
        }
        match addr {
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
        (addr, value)
    }
    fn write(&mut self, (addr, value): Self::WriteValue) {
        match addr {
            SFR_P2 => self.p2 = value,
            SFR_P3 => self.p3 = value,
            _ => unreachable!(),
        }
    }
}

pub struct DiagnosticMonitor {
    ram: [u8; 256],
}

impl Default for DiagnosticMonitor {
    fn default() -> Self {
        Self { ram: [0; 256] }
    }
}

impl PortMapper for DiagnosticMonitor {
    type WriteValue = (u8, u8);
    fn interest<C: CpuView>(&self, cpu: &C, addr: u8) -> bool {
        addr == 0x1f || addr == 0x7e
    }
    fn read<C: CpuView>(&self, cpu: &C, addr: u8) -> u8 {
        self.ram[addr as usize]
    }
    fn prepare_write<C: CpuView>(&self, cpu: &C, addr: u8, value: u8) -> Self::WriteValue {
        trace!(
            "Diagnostic write {addr:02X?} = {value:02X?} @ {:04X}",
            cpu.pc_ext()
        );
        (addr, value)
    }
    fn write(&mut self, value: Self::WriteValue) {
        self.ram[value.0 as usize] = value.1;
    }
}

pub struct RAM {
    pub sram: [u8; 0x8000],  // 32kB
    pub vram: [u8; 0x20000], // 128kB
    pub mapper: Mapper,
    pub peripheral: [u8; 0x100],
    pub rom_bank: Rc<Cell<bool>>,
    pub input_queue: RefCell<Vec<u8>>,
    pub sync: SyncHolder,
    pub nvr: Nvr,
    pub duart_input: u8,
    pub duart_output_inv: u8,
}

impl RAM {
    pub fn new(rom_bank: Rc<Cell<bool>>, sync: SyncHolder) -> Self {
        let sram = [0; 0x8000];
        let vram = [0; 0x20000];
        let mapper = Mapper::new();
        let peripheral = [0; 0x100];
        Self {
            sram,
            vram,
            mapper,
            peripheral,
            rom_bank,
            input_queue: RefCell::new("x".to_string().into_bytes()),
            sync,
            nvr: Nvr::new(),
            duart_input: 0,
            duart_output_inv: 0,
        }
    }
}

fn swizzle_video_ram(addr: u16, bits: u8) -> u16 {
    if bits & 0x10 == 0 {
        addr
    } else {
        // E O E O -> O E E O
        // 2 3 4 5 -> 3 2 4 5 ?
        if (0x200..0x400).contains(&addr) {
            addr ^ 0x0100
        } else {
            addr
        }
    }
}

fn calculate_mapper_7ff6(a: u8, b: u8, vram: &[u8]) -> u8 {
    const C: [u8; 16] = [
        0x0b, 0x0b, 0x0b, 0x0d, 0x0b, 0x04, 0x0b, 0x0d, 0x03, 0x03, 0x03, 0x0d, 0x03, 0x01, 0x03,
        0x0d,
    ];

    let c4 = (a & 0b0000_1000) != 0;
    let x = if c4 { b } else { a };

    let c0 = (b & 0b0000_1000) != 0; // force double?
    let c1 = (a & 0b0100_0000) != 0; // ?
    let c2 = (x & 0b0000_0010) != 0; // width/invert?
    let c3 = (x & 0b0000_0001) != 0; // width/invert?

    let c_idx = c0 as u8 | ((c1 as u8) << 1) | ((c2 as u8) << 2) | ((c3 as u8) << 3);
    let c = C[c_idx as usize];

    // Expected output from the mapper when we place a '2' in the second field for a row,
    // indexed by row
    let expected: [u8; 26] =
        hex!("04 06 08 0a 0c 0e 0f 00 01 02 03 05 07 09 0b 0d 0e 0f 00 01 02 04 06 08 0a 0c");
    if vram[1] == 0 || vram[1] == 2 {
        let check = &vram[1..expected.len() * 2 + 2];
        if let Some(pos) = check.iter().position(|&x| x == 2) {
            return expected[pos / 2];
        }
    }

    // This isn't totally correct, it seems to require a function of all rows
    let mask_bits = match vram[1] & 0b0000_1111 {
        0b0000 => 0b0000,
        0b0100 => 0b1110,
        0b1000 => 0b1011,
        0b1100 => 0b0001,
        _ => 0b0000,
    };

    trace!(
        "RAM A: {:02X?} {a:08b}, B: {:02X?} {b:08b}, C[{:02X?}] = {:02X?} {c:08b} mask: {:02X?}={mask_bits:08b}",
        a, b, c_idx, c, vram[1]
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
    fn target_for_addr(&self, mut addr: u16) -> (MemoryTarget, u32) {
        if (0x7ff0..=0x7fff).contains(&addr) {
            (MemoryTarget::Mapper, (addr & 0x0f) as u32)
        } else if (0x7fe0..=0x7fef).contains(&addr) {
            (MemoryTarget::DUART, (addr & 0x0f) as u32)
        } else if (0x7e00..=0x7eff).contains(&addr) && self.mapper.get(3) & 0x04 == 0 {
            (MemoryTarget::Peripheral, (addr & 0x0ff) as u32)
        } else if addr < 0x8000 {
            let vram_offset = 0;
            if (0x200..0x600).contains(&addr) {
                addr = swizzle_video_ram(addr, self.mapper.get(3));
            }
            (MemoryTarget::VRAM, vram_offset + addr as u32)
        } else {
            if self.mapper.vram_page() == 1 {
                // if self.sram_mapped() == 1 {
                //     (MemoryTarget::VRAM, (addr - 0x8000) as u32)
                // } else {
                (MemoryTarget::VRAM, (addr) as u32)
                // }
            } else {
                (MemoryTarget::SRAM, (addr - 0x8000) as u32)
            }
        }
    }

    pub fn tick(&mut self) {
        let nvrtxd = self.duart_output_inv & 1 << 6 == 0;
        let nvrclk = self.duart_output_inv & 1 << 5 == 0;
        let nvrcs = self.duart_output_inv & 1 << 4 == 0;
        let (nvrrxd, nvrrdy) = self.nvr.tick(nvrcs, nvrclk, nvrtxd);

        self.duart_input = self.duart_input & !(1 << 4) | (nvrrdy as u8) << 4;
        self.duart_input = self.duart_input & !(1 << 3) | (nvrrxd as u8) << 3;
    }
}

impl MemoryMapper for RAM {
    type WriteValue = (MemoryTarget, u32, u32, u32, u8);
    fn len(&self) -> u32 {
        self.sram.len() as u32 + self.vram.len() as u32
    }
    fn read<C: CpuView>(&self, cpu: &C, addr: u32) -> u8 {
        let pc = cpu.pc_ext();
        let mut addr = addr as u16;

        let (target, offset) = self.target_for_addr(addr);
        match target {
            MemoryTarget::Mapper => match offset {
                0x6 => {
                    if tracing::enabled!(tracing::Level::TRACE) {
                        trace!("VIDEO VRAM addr = {:02X?}", &self.vram[0..60]);
                    }
                    calculate_mapper_7ff6(self.mapper.get(3), self.mapper.get(4), &self.vram)
                }
                x => self.mapper.get(x as _),
            },
            MemoryTarget::DUART => {
                trace!(
                    "DUART RAM read from {} @ {:05X}",
                    READ_2681[offset as usize], pc
                );

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
                    return self.duart_input;
                }
                if addr == 0x7feb {
                    let next = self.input_queue.borrow_mut().remove(0);
                    trace!("RAM Next: {:?}", next as char);
                    return next;
                }

                return 0;
            }
            MemoryTarget::Peripheral => {
                trace!(
                    "Peripheral read: 0x{:04X} = 0x{:02X} @ {:05X}",
                    addr, self.peripheral[offset as usize], pc
                );
                // peripheral
                return self.peripheral[offset as usize];
            }
            MemoryTarget::VRAM => {
                trace!(
                    "VRAM read: 0x{:04X} = 0x{:02X} @ {:05X}",
                    addr, self.vram[offset as usize], pc
                );
                return self.vram[offset as usize];
            }
            MemoryTarget::SRAM => {
                trace!(
                    "SRAM read: 0x{:04X} = 0x{:02X} @ {:05X}",
                    addr, self.sram[offset as usize], pc
                );
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
        // if pc != 0x15B33 && pc != 0x15BB1 && pc != 0x15BCC {
        //     trace!(
        //         "RAM write: 0x{:04X} = 0x{:02X} ({:?}) @ {pc:05X}",
        //         addr, value, value as char
        //     );
        // }

        match target {
            MemoryTarget::Mapper => {
                info!(
                    "Mapper write: 0x{:04X} = 0x{:02X} -> 0x{:02X} @ {pc:05X}",
                    addr,
                    self.mapper.get(offset as _),
                    value
                );
                if offset == 0x3
                    && self.mapper.sram_mapped() ^ self.mapper.sram_mapped_value(value) != 0
                {
                    let old = self.mapper.sram_mapped();
                    let new = self.mapper.sram_mapped_value(value);
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

                if offset == 0x4 {
                    self.sync.set_hz_70((value & 0x10) != 0);
                }

                self.mapper.set(offset as _, value);
            }
            MemoryTarget::DUART => {
                trace!(
                    "DUART write: {} 0x{:04X} = 0x{:02X}",
                    WRITE_2681[offset as usize], addr, value
                );
                if offset == 0xe {
                    self.duart_output_inv |= value;
                }
                if offset == 0xf {
                    self.duart_output_inv &= !value;
                }
            }
            MemoryTarget::Peripheral => {
                trace!("Peripheral write: 0x{:04X} = 0x{:02X}", addr, value);
                self.peripheral[offset as usize] = value;
            }
            MemoryTarget::VRAM => {
                trace!("VRAM write: 0x{:04X} = 0x{:02X} @ {:05X}", addr, value, pc);
                self.vram[offset as usize] = value;
            }
            MemoryTarget::SRAM => {
                trace!("SRAM write: 0x{:04X} = 0x{:02X} @ {:05X}", addr, value, pc);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// It's not clear what the mapper is doing, so let's just test we output
    /// the same values as the ROM expects.
    #[test]
    fn test_calculate_mapper_7ff6() {
        // The offsets for each row - remember that this is shifted left by 1 when stored
        // in ram.
        const ROWS: [u8; 27] = hex!(
            "01 02 04 08 05 10 20 40 50 70 11 22 44 2a 55 03 06 0c 18 30 60 07 0e 1c 38 0f 1e"
        );

        let mut vram = [0_u8; 0x40];
        for (i, &row) in ROWS.iter().enumerate() {
            vram[i * 2] = row << 1;
        }
        eprintln!("vram = {:02X?}", vram);

        // Set 7ff3/7ff4 to various values, with the second field set to zero
        const EXPECTED_0: [u8; 32] = hex!(
            "0b 0b 0b 0d 0b 04 0b 0d 03 03 03 0d 03 01 03 0d 0b 0b 0b 0d 0b 04 0b 0d 03 03 03 0d 03 01 03 0d"
        );
        let mut mapper3 = 0;
        let mut mapper4 = 0;
        for i in 0..32 {
            let i2 = (i & (1 << 2)) != 0;
            let i3 = (i & (1 << 3)) != 0;
            mapper3 &= 0b10111111;
            if (i & (1 << 1)) != 0 {
                mapper3 |= 0b01000000;
            }
            mapper3 |= 0b00001000;
            if (i & (1 << 4)) != 1 {
                mapper3 = (mapper3 & 0b11110100) | (i3 as u8) | ((i2 as u8) << 1);
            }
            mapper4 &= 0b11110111;
            if (i & (1 << 0)) != 0 {
                mapper4 |= 0b00001000;
            }
            if (i & (1 << 4)) != 0 {
                mapper4 = (mapper4 & 0b11111100) | (i3 as u8) | ((i2 as u8) << 1);
            }

            let result = calculate_mapper_7ff6(mapper3, mapper4, &vram);
            eprintln!(
                "i = {:02X?}, a = {:02X?}, b = {:02X?}, result = {:02X?}",
                i, mapper3, mapper4, result
            );
            assert_eq!(result, EXPECTED_0[i], "vram = {:02X?}", vram);
        }

        // Set the second field of all rows to 0x0c, 0x08, 0x04, 0x00
        const EXPECTED_1: [u8; 4] = hex!("0a 00 05 0b");
        for (i, &v) in [0x0c, 0x08, 0x04, 0].iter().enumerate() {
            let mapper3 = 4;
            let mapper4 = 0x1b;

            for j in 0..vram.len() {
                if j % 2 == 1 {
                    vram[j] = v;
                }
            }

            let result = calculate_mapper_7ff6(mapper3, mapper4, &vram);
            assert_eq!(result, EXPECTED_1[i], "vram = {:02X?}", vram);
        }

        // Set bit 1 of a single field at a time, starting from the second last (ie: 0x0f in the list of ROWS above)
        const EXPECTED_2: [u8; 26] =
            hex!("04 06 08 0a 0c 0e 0f 00 01 02 03 05 07 09 0b 0d 0e 0f 00 01 02 04 06 08 0a 0c");
        for i in (0..26).rev() {
            vram[i * 2 + 1] ^= 2;
            vram[i * 2 + 3] = 0;

            let result = calculate_mapper_7ff6(mapper3, mapper4, &vram);
            assert_eq!(result, EXPECTED_2[i], "vram = {:02X?}", vram);
        }
    }
}
