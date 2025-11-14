use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;

use i8051::sfr::SFR_P1;
use i8051::sfr::SFR_P2;
use i8051::sfr::SFR_P3;
use i8051::{CpuView, MemoryMapper, PortMapper, ReadOnlyMemoryMapper};
use tracing::debug;
use tracing::{info, trace};

use crate::machine::generic::duart::{DUART, ReadRegister, WriteRegister};
use crate::machine::generic::nvr::Nvr;
use crate::machine::generic::vsync::SyncGen;
use crate::machine::vt420::video::{Mapper, TIMING_60HZ, TIMING_70HZ};

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
    pub p1: u8,
    pub p1_read: u8,
    pub p2: u8,
    pub p3: u8,
    pub p3_read: u8,
    pub sync: SyncHolder,
}

impl VideoProcessor {
    pub fn new() -> Self {
        Self {
            p1: 0b1111_1111,
            // Bit 0-3: 0 for rotation disable, 1 for rotation enable
            // Bit 6: 1 for enable 232/423 select (ie: mux to 423)
            p1_read: 0b1111_1111,
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
        addr == SFR_P2 || addr == SFR_P3 || addr == SFR_P1
    }
    fn read<C: CpuView>(&self, cpu: &C, addr: u8) -> u8 {
        if addr == SFR_P3 {
            // trace!("P3 read {:02X} @ {:X}", self.p3_read, cpu.pc_ext());
        }
        match addr {
            SFR_P1 => self.p1_read,
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
            SFR_P1 => self.p1,
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
        if addr == SFR_P1 {
            trace!("P1 write {:02X} @ {:X}", value, cpu.pc_ext());
        }
        (addr, value)
    }
    fn write(&mut self, (addr, value): Self::WriteValue) {
        match addr {
            SFR_P1 => self.p1 = value,
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
    pub sram: Box<[u8; 0x8000]>,  // 32kB
    pub vram: Box<[u8; 0x20000]>, // 128kB
    pub mapper: Mapper,
    pub peripheral: [u8; 0x100],
    pub rom_bank: Rc<Cell<bool>>,
    pub input_queue: RefCell<Vec<u8>>,
    pub sync: SyncHolder,
    pub nvr: Nvr,
    pub duart: DUART,
}

impl RAM {
    pub fn new(rom_bank: Rc<Cell<bool>>, sync: SyncHolder, duart: DUART) -> Self {
        let sram = Box::new([0; 0x8000]);
        let vram = Box::new([0; 0x20000]);
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
            duart,
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
        } else if (0x7e00..=0x7eff).contains(&addr) {
            //&& self.mapper.get(3) & 0x04 == 0 {
            (MemoryTarget::Peripheral, (addr & 0x0ff) as u32)
        } else if addr < 0x8000 {
            if (0x200..0x400).contains(&addr) {
                addr = swizzle_video_ram(addr, self.mapper.get(3));
            }
            let vram_offset = self.mapper.vram_offset_0();
            (MemoryTarget::VRAM, vram_offset + addr as u32)
        } else {
            let addr = (addr & 0x7fff) as u32;
            if self.mapper.map_vram_at_8000() == 1 {
                let vram_offset = self.mapper.vram_offset();
                (MemoryTarget::VRAM, addr + vram_offset)
            } else {
                (MemoryTarget::SRAM, addr)
            }
        }
    }

    pub fn tick(&mut self) {
        let nvrtxd = self.duart.output_bits_inv & 1 << 6 == 0;
        let nvrclk = self.duart.output_bits_inv & 1 << 5 == 0;
        let nvrcs = self.duart.output_bits_inv & 1 << 4 == 0;
        let (nvrrxd, nvrrdy) = self.nvr.tick(nvrcs, nvrclk, nvrtxd);

        self.duart.input_bits = self.duart.input_bits & !(1 << 4) | (nvrrdy as u8) << 4;
        self.duart.input_bits = self.duart.input_bits & !(1 << 3) | (nvrrxd as u8) << 3;

        let int1 = self.duart.tick();
    }
}

impl MemoryMapper for RAM {
    type WriteValue = (MemoryTarget, u32, u32, u32, u8);
    fn len(&self) -> u32 {
        self.sram.len() as u32 + self.vram.len() as u32
    }
    fn read<C: CpuView>(&self, cpu: &C, addr: u32) -> u8 {
        let pc = cpu.pc_ext();
        let addr = addr as u16;

        let (target, offset) = self.target_for_addr(addr);
        match target {
            MemoryTarget::Mapper => match offset {
                0x6 => {
                    if tracing::enabled!(tracing::Level::TRACE) {
                        debug!("VIDEO VRAM addr = {:02X?}", &self.vram[0..60]);
                    }
                    self.mapper.read_7ff6(self.vram.as_ref())
                }
                x => self.mapper.get(x as _),
            },
            MemoryTarget::DUART => {
                let read = ReadRegister::try_from(offset as u8).unwrap();
                let value = self.duart.read(read);
                debug!("DUART read {read:?} = {:02X} @ {:05X}", value, pc);
                value
            }
            MemoryTarget::Peripheral => {
                debug!(
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
                debug!(
                    "Mapper write: 0x{:04X} = 0x{:02X} -> 0x{:02X} @ {pc:05X}",
                    addr,
                    self.mapper.get(offset as _),
                    value
                );
                if offset == 0x3
                    && self.mapper.vram_8000_bit() ^ self.mapper.vram_8000_bit_value(value) != 0
                {
                    let old = self.mapper.vram_8000_bit();
                    let new = self.mapper.vram_8000_bit_value(value);
                    debug!("VIDEO: VRAM page changed: {} -> {}", old, new);
                    // if old == 1 && new == 0 {
                    //     let font = &self.vram[0..];
                    //     std::fs::write("/tmp/font.bin", font).unwrap();
                    // }
                }

                if offset == 0x5 {
                    debug!("Memory mapper bank write: {:02X}", value);
                    let bank = (value & 0x4) != 0;
                    if bank != self.rom_bank.get() {
                        debug!("RAM write bank changed: {}", bank as u8);
                        self.rom_bank.set(bank);
                    }
                }

                if offset == 0x4 {
                    self.sync.set_hz_70((value & 0x10) != 0);
                }

                self.mapper.set(offset as _, value);
            }
            MemoryTarget::DUART => {
                let reg = WriteRegister::try_from(offset as u8).unwrap();
                debug!("DUART write {reg:?} = {:02X} @ {:05X}", value, pc);
                self.duart.write(reg, value);
            }
            MemoryTarget::Peripheral => {
                debug!("Peripheral write: 0x{:04X} = 0x{:02X}", addr, value);
                self.peripheral[offset as usize] = value;
            }
            MemoryTarget::VRAM => {
                debug!("VRAM write: 0x{:04X} = 0x{:02X} @ {:05X}", addr, value, pc);
                self.vram[offset as usize] = value;
            }
            MemoryTarget::SRAM => {
                debug!("SRAM write: 0x{:04X} = 0x{:02X} @ {:05X}", addr, value, pc);
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
}

impl ROM {
    /// Create a new memory mapper with ROM loaded from file
    /// Bank 0: first 64KB of ROM, Bank 1: remaining ROM data
    /// Initializes with bank 0 mapped
    pub fn new(rom: Vec<u8>) -> Self {
        let rom_size = rom.len();
        let bank_size = 0x10000; // 64KB per bank

        Self {
            rom,
            rom_size,
            bank_size,
        }
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
