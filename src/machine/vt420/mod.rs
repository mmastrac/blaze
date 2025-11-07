pub mod breakpoints;
pub mod memory;
pub mod video;

use std::cell::Cell;
use std::fs;
use std::mem;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use hex_literal::hex;
use i8051::breakpoint::Breakpoints;
use i8051::peripheral::{P3_INT1, Serial, Timer};
use i8051::{Cpu, CpuContext, CpuView, DefaultPortMapper, PortMapper};
use tracing::{info, trace, warn};

use crate::host::comm::{self, CommConfig};
use crate::machine::generic::duart::DUART;
use crate::machine::generic::lk201::LK201;
use crate::machine::vt420::video::decode_vram;

use self::memory::{Bank, DiagnosticMonitor, RAM, ROM, VideoProcessor};

#[cfg(feature = "pc-trace")]
use bit_set::BitSet;

pub(crate) struct System {
    pub(crate) rom: ROM,
    pub(crate) memory: RAM,
    bank: Bank,
    nvr_file: Option<PathBuf>,
    nvr_write: usize,

    video_row: VideoProcessor,
    serial: Serial,
    diagnostic_monitor: DiagnosticMonitor,
    timer: Timer,
    default: DefaultPortMapper,
    dtr_a: Rc<Cell<bool>>,
    dtr_b: Rc<Cell<bool>>,

    pub(crate) keyboard: LK201,
    pub(crate) breakpoints: Breakpoints,

    #[cfg(feature = "pc-trace")]
    pub(crate) pc_bitset: BitSet,
    #[cfg(feature = "pc-trace")]
    pub(crate) pc_bitset_current: BitSet,
}

impl System {
    pub(crate) fn new(
        rom: &Path,
        nvr: Option<&Path>,
        comm1: CommConfig,
        comm2: CommConfig,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let bank = Bank::default();
        info!("Loading ROM into memory...");
        let rom = ROM::new(&rom, bank.bank.clone())?;
        let video_row = VideoProcessor::new();
        let (serial, in_kbd, out_kbd) = Serial::new(60);
        let (duart, channel_a, channel_b) = DUART::new();

        let dtr_a = comm::connect_duart(channel_a, comm1)?;
        let dtr_b = comm::connect_duart(channel_b, comm2)?;

        let mut memory = RAM::new(bank.bank.clone(), video_row.sync.clone(), duart);
        let mut nvr_file = None;
        if let Some(nvr) = nvr {
            nvr_file = Some(nvr.to_owned());
            if !nvr.exists() {
                warn!("NVR file does not exist, creating it");
                fs::write(nvr, vec![0xff; 128])?;
            }
            let mut nvr = fs::read(nvr)?;
            if nvr.len() < 128 {
                warn!("NVR file is too small, padding with zeros");
                nvr.resize(128, 0xff);
            } else if nvr.len() > 128 {
                warn!("NVR file is too large, truncating");
                nvr.truncate(128);
            }
            memory.nvr.mem.copy_from_slice(&nvr);
        } else {
            // Some checksums hand-modified (0x30, 0x50, 0x70) for tests to pass
            let initial_nvr = hex!(
                "65 44 88 1e 1e 85 54 88  85 54 00 00 04 50 00 00"
                "00 00 00 00 00 00 00 00  00 00 00 00 00 00 00 00"
                "00 00 00 00 00 00 00 00  00 00 00 00 00 00 00 00"
                "03 00 c0 25 00 24 01 00  00 00 02 98 00 00 00 00"
                "01 01 01 01 01 01 01 01  01 01 01 01 01 01 01 01"
                "4a 00 c0 25 00 24 01 00  00 00 02 98 00 00 00 00"
                "01 01 01 01 01 01 01 01  01 01 01 01 01 01 01 01"
                "4a ff ff ff ff ff ff ff  ff ff ff ff ff ff ff ff"
            );

            memory.nvr.mem.fill(0xff);
            memory.nvr.mem[..initial_nvr.len()].copy_from_slice(&initial_nvr);
        }
        Ok(Self {
            bank,
            memory,
            rom,
            nvr_file,
            nvr_write: 0,
            video_row,
            serial,
            dtr_a,
            dtr_b,
            diagnostic_monitor: DiagnosticMonitor::default(),
            timer: Timer::default(),
            default: DefaultPortMapper::default(),
            keyboard: LK201::new(in_kbd.clone(), out_kbd),
            breakpoints: Breakpoints::new(),
            #[cfg(feature = "pc-trace")]
            pc_bitset: BitSet::with_capacity(0x10000),
            #[cfg(feature = "pc-trace")]
            pc_bitset_current: BitSet::with_capacity(0x10000),
        })
    }

    pub(crate) fn step(&mut self, cpu: &mut Cpu) {
        let start = Instant::now();
        let mut breakpoints = Breakpoints::default();
        mem::swap(&mut self.breakpoints, &mut breakpoints);
        breakpoints.run(true, cpu, self);
        mem::swap(&mut self.breakpoints, &mut breakpoints);

        let prev_0x1f = cpu.internal_ram[0x1f];
        cpu.step(self);
        let new_0x1f = cpu.internal_ram[0x1f];
        if prev_0x1f != new_0x1f {
            info!(
                "0x1f changed from {prev_0x1f:02X} to {new_0x1f:02X} @ {:04X}",
                cpu.pc_ext(self)
            );
        }

        #[cfg(feature = "pc-trace")]
        {
            self.pc_bitset.insert(cpu.pc_ext(self) as usize);
        }

        self.memory.tick();
        self.keyboard.tick();
        self.serial.tick(cpu);
        let prev_p3 = self.video_row.p3_read;
        self.video_row.p3_read &= !P3_INT1;
        if !self.memory.duart.interrupt {
            self.video_row.p3_read |= P3_INT1;
            if prev_p3 & P3_INT1 == 0 {
                trace!("DUART interrupt cleared");
            }
        } else if prev_p3 & P3_INT1 != 0 {
            trace!("DUART interrupt");
        }
        let dtr_a = self.memory.duart.output_bits_inv & (1 << 1) != 0;
        let dtr_b = self.memory.duart.output_bits_inv & (1 << 7) != 0;
        if self.dtr_a.replace(dtr_a) != dtr_a {
            trace!("DUART pipe A DTR changed to {}", self.dtr_a.get());
        }
        if self.dtr_b.replace(dtr_b) != dtr_b {
            trace!("DUART pipe B DTR changed to {}", self.dtr_b.get());
        }
        self.video_row.tick();
        let tick = self.timer.prepare_tick(cpu, self);
        self.timer.tick(cpu, tick);

        if self.memory.nvr.write_count > self.nvr_write {
            if let Some(nvr_file) = &self.nvr_file {
                fs::write(nvr_file, self.memory.nvr.mem).unwrap();
            }
            self.nvr_write = self.memory.nvr.write_count;
        }

        mem::swap(&mut self.breakpoints, &mut breakpoints);
        breakpoints.run(false, cpu, self);
        mem::swap(&mut self.breakpoints, &mut breakpoints);
        if start.elapsed() > Duration::from_millis(100) {
            warn!("Step took too long: {:?}", start.elapsed());
        }
    }

    pub(crate) fn dump_screen_text(&self) -> String {
        let text = String::with_capacity(132 * 25);
        decode_vram(
            &self.memory.vram,
            &self.memory.mapper,
            |text, _, _| {
                text.push_str("\n");
            },
            |text, _col, ch, _attrs| {
                text.push(ch);
            },
            text,
        )
    }
}

impl PortMapper for System {
    type WriteValue = <(
        VideoProcessor,
        (Serial, (DiagnosticMonitor, (Timer, DefaultPortMapper))),
    ) as PortMapper>::WriteValue;
    fn interest<C: CpuView>(&self, cpu: &C, addr: u8) -> bool {
        (
            &self.video_row,
            (
                &self.serial,
                (&self.diagnostic_monitor, (&self.timer, &self.default)),
            ),
        )
            .interest(cpu, addr)
    }
    fn read<C: CpuView>(&self, cpu: &C, addr: u8) -> u8 {
        (
            &self.video_row,
            (
                &self.serial,
                (&self.diagnostic_monitor, (&self.timer, &self.default)),
            ),
        )
            .read(cpu, addr)
    }
    fn prepare_write<C: CpuView>(&self, cpu: &C, addr: u8, value: u8) -> Self::WriteValue {
        (
            &self.video_row,
            (
                &self.serial,
                (&self.diagnostic_monitor, (&self.timer, &self.default)),
            ),
        )
            .prepare_write(cpu, addr, value)
    }
    fn write(&mut self, value: Self::WriteValue) {
        (
            &mut self.video_row,
            (
                &mut self.serial,
                (
                    &mut self.diagnostic_monitor,
                    (&mut self.timer, &mut self.default),
                ),
            ),
        )
            .write(value)
    }
    fn extend_short_read<C: CpuView>(&self, cpu: &C, addr: u8) -> u16 {
        (
            &self.video_row,
            (
                &self.serial,
                (&self.diagnostic_monitor, (&self.timer, &self.default)),
            ),
        )
            .extend_short_read(cpu, addr)
    }
    fn pc_extension<C: CpuView>(&self, cpu: &C) -> u16 {
        self.bank.pc_extension(cpu)
    }
    fn read_latch<C: CpuView>(&self, cpu: &C, addr: u8) -> u8 {
        (
            &self.video_row,
            (
                &self.serial,
                (&self.diagnostic_monitor, (&self.timer, &self.default)),
            ),
        )
            .read_latch(cpu, addr)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_boots() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let mut system = System::new(
            &Path::new(&format!("{}/roms/vt420/23-068E9-00.bin", manifest_dir)),
            None,
            CommConfig::default(),
            CommConfig::default(),
        )
        .unwrap();
        let mut cpu = Cpu::new();
        for _ in 0..9850880 {
            system.step(&mut cpu);
        }

        let screen = system.dump_screen_text();
        assert!(screen.contains("VT420 OK"), "{screen}");
    }
}
