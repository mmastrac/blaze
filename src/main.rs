use bit_set::BitSet;
use clap::Parser;
use hex_literal::hex;
use i8051::breakpoint::{Action, Breakpoints};
use i8051::peripheral::{P3_INT1, Serial, Timer};
use i8051::sfr::{SFR_P1, SFR_P2, SFR_P3};
use ratatui::crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::layout::Offset;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use std::cell::Cell;
use std::fs::{self, File};
use std::io::{self, IsTerminal, stdout};
use std::mem;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};
use tracing::{Level, info, trace, warn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm;

mod comm;
mod duart;
mod lk201;
mod memory;
mod nvr;
mod screen;
mod ssu;
mod video;

use memory::{Bank, RAM, ROM, VideoProcessor};

use i8051::{Cpu, CpuContext, CpuView, DefaultPortMapper, PortMapper};

use crate::comm::CommConfig;
use crate::duart::DUART;
use crate::lk201::{LK201, SpecialKey};
use crate::memory::DiagnosticMonitor;
use crate::screen::{DisplayMode, Screen};

/// VT420 Terminal Emulator
/// Emulates a VT420 terminal using an 8051 microcontroller
#[derive(Parser)]
#[command(name = "vt-emulator")]
#[command(about = "A VT420 terminal emulator using 8051 CPU emulation")]
struct Args {
    /// Path to the ROM file
    #[arg(long)]
    rom: PathBuf,

    /// Path to the non-volatile RAM file
    #[arg(long)]
    nvr: Option<PathBuf>,

    /// Display the video output
    #[arg(long)]
    display: bool,

    /// Comm1: Single bidirectional pipe
    #[arg(long = "comm1-pipe", value_name = "PIPE")]
    comm1_pipe: Option<PathBuf>,

    /// Comm1: Separate read and write pipes
    #[arg(long = "comm1-pipes", num_args = 2, value_names = ["RX", "TX"])]
    comm1_pipes: Vec<PathBuf>,

    /// Comm1: Execute a command and connect to its pty
    #[arg(long = "comm1-exec", value_name = "COMMAND")]
    comm1_exec: Option<String>,

    /// Comm2: Single bidirectional pipe
    #[arg(long = "comm2-pipe", value_name = "PIPE")]
    comm2_pipe: Option<PathBuf>,

    /// Comm2: Separate read and write pipes
    #[arg(long = "comm2-pipes", num_args = 2, value_names = ["RX", "TX"])]
    comm2_pipes: Vec<PathBuf>,

    /// Comm2: Execute a command and connect to its pty
    #[arg(long = "comm2-exec", value_name = "COMMAND")]
    comm2_exec: Option<String>,

    /// Display the video RAM
    #[arg(long, requires = "display")]
    show_vram: bool,

    /// Display the mapper
    #[arg(long, requires = "display")]
    show_mapper: bool,

    /// Enable debugger
    #[arg(long)]
    debug: bool,

    /// Breakpoints for debug mode, repeatable, parsed as hex
    #[arg(value_parser = parse_hex_address, long="bp", alias="breakpoint")]
    breakpoint: Vec<u32>,

    /// Enable instruction tracing
    #[arg(long)]
    trace: bool,

    /// Enable verbose output
    #[arg(short, long)]
    verbose: bool,
}

fn parse_hex_address(s: &str) -> Result<u32, Box<dyn std::error::Error + Send + Sync>> {
    Ok(u32::from_str_radix(s, 16)?)
}

struct System {
    rom: ROM,
    memory: RAM,
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

    keyboard: LK201,
    breakpoints: Breakpoints,

    #[cfg(feature = "pc-trace")]
    pc_bitset: BitSet,
    #[cfg(feature = "pc-trace")]
    pc_bitset_current: BitSet,
}

impl System {
    fn new(
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

        let dtr_a = crate::comm::connect_duart(channel_a, comm1)?;
        let dtr_b = crate::comm::connect_duart(channel_b, comm2)?;

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

    fn step(&mut self, cpu: &mut Cpu) {
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let level = if args.verbose {
        Level::TRACE
    } else {
        Level::INFO
    };
    if args.display {
        tracing_subscriber::fmt()
            .with_max_level(level)
            .with_ansi(false)
            .with_writer(File::create("/tmp/blaze-vt.log").unwrap())
            .init();
    } else if !args.debug {
        let format = tracing_subscriber::fmt::format()
            .with_target(false)
            .with_line_number(false)
            .with_level(false)
            .without_time();
        if stdout().is_terminal() {
            tracing_subscriber::fmt()
                .with_max_level(level)
                .event_format(format)
                .log_internal_errors(false)
                .init();
        } else {
            tracing_subscriber::fmt()
                .with_ansi(false)
                .with_max_level(level)
                .event_format(format)
                .log_internal_errors(false)
                .init();
        }
    }

    info!("VT420 Emulator starting...");
    info!("ROM file: {:?}", args.rom);

    // Check if ROM file exists
    if !args.rom.exists() {
        info!("Error: ROM file does not exist: {:?}", args.rom);
        std::process::exit(1);
    }

    // Initialize the 8051 CPU emulator
    info!("Initializing 8051 CPU emulator...");
    let mut cpu = Cpu::new();

    // Start CPU execution
    info!("Starting CPU execution...");
    let start_time = Instant::now();
    let mut instruction_count = 0;

    // Parse comm1 configuration
    let comm1_pipes = if args.comm1_pipes.len() == 2 {
        Some((args.comm1_pipes[0].clone(), args.comm1_pipes[1].clone()))
    } else {
        None
    };
    let comm1_config = CommConfig::from_args(args.comm1_pipe, comm1_pipes, args.comm1_exec);

    // Parse comm2 configuration
    let comm2_pipes = if args.comm2_pipes.len() == 2 {
        Some((args.comm2_pipes[0].clone(), args.comm2_pipes[1].clone()))
    } else {
        None
    };
    let comm2_config = CommConfig::from_args(args.comm2_pipe, comm2_pipes, args.comm2_exec);

    let mut system =
        System::new(&args.rom, args.nvr.as_deref(), comm1_config, comm2_config).unwrap();

    // Enable tracing if requested
    // if args.trace && args.verbose {
    //     info!("Instruction tracing enabled");
    //     breakpoints.add(true, 0, Action::SetTraceInstructions(true));
    //     breakpoints.add(true, 0x10000, Action::SetTraceInstructions(true));
    // }

    let breakpoints = &mut system.breakpoints;
    let code = &system.rom;
    for addr in code.find_bank_dispatch() {
        breakpoints.add(
            true,
            addr.dispatch_addr,
            Action::Log(format!(
                "Calling bank {}/{:X}h @ {:X}h",
                addr.target_addr >> 16,
                addr.id,
                addr.target_addr
            )),
        );
        breakpoints.add(
            true,
            addr.target_addr,
            Action::Log(format!(
                "Entered bank {}/{:X}h @ {:X}h",
                addr.target_addr >> 16,
                addr.id,
                addr.target_addr
            )),
        );
    }

    breakpoints.add(true, 0x0, Action::Log("Interrupt: CPU reset".to_string()));
    breakpoints.add(
        true,
        0x10000,
        Action::Log("Interrupt: CPU reset".to_string()),
    );
    breakpoints.add(true, 0xB, Action::Log("Interrupt: Timer0".to_string()));
    breakpoints.add(true, 0x10000, Action::Log("Interrupt: Timer0".to_string()));

    breakpoints.add(
        true,
        0xb66,
        Action::Log("Interrupt: Entering user code".to_string()),
    );
    breakpoints.add(true, 0xb66, Action::TraceRegisters);
    breakpoints.add(
        true,
        0xc30,
        Action::Log("Interrupt: Leaving user code".to_string()),
    );
    breakpoints.add(true, 0xc30, Action::TraceRegisters);
    breakpoints.add(
        true,
        0x10b66,
        Action::Log("Interrupt:Entering user code".to_string()),
    );
    breakpoints.add(true, 0x10b66, Action::TraceRegisters);
    breakpoints.add(
        true,
        0x10c30,
        Action::Log("Interrupt: Leaving user code".to_string()),
    );
    breakpoints.add(true, 0x10c30, Action::TraceRegisters);

    breakpoints.add(true, 0x5a88, Action::Log("Test failed!!!".to_string()));
    breakpoints.add(true, 0x5d5a, Action::Log("Testing failed!!!".to_string()));

    breakpoints.add(
        true,
        0x15ed1,
        Action::Log("Testing keyboard serial loopback".to_string()),
    );

    breakpoints.add(
        true,
        0x16153,
        Action::Log("Testing keyboard serial".to_string()),
    );

    breakpoints.add(
        true,
        0x100b,
        Action::Log("KBD: Command requires ACK".to_string()),
    );

    breakpoints.add(
        true,
        0x1100b,
        Action::Log("KBD: Command requires ACK".to_string()),
    );

    breakpoints.add(true, 0x1009, Action::Log("KBD: Got ack".to_string()));

    breakpoints.add(true, 0x11009, Action::Log("KBD: Got ack".to_string()));

    // breakpoints.add(true, 0xf0d, Action::Log("Update DUART bits".to_string()));
    // breakpoints.add(true, 0x10f0d, Action::Log("Update DUART bits".to_string()));

    breakpoints.add(true, 0x15ad0, Action::Log("Testing ROM Bank 1".to_string()));
    breakpoints.add(true, 0x20ca, Action::Log("Testing ROM Bank 0".to_string()));
    breakpoints.add(true, 0x15aeb, Action::Log("Testing phase 2".to_string()));
    breakpoints.add(true, 0x15b23, Action::Log("RAM test".to_string()));
    breakpoints.add(true, 0x15b8a, Action::Log("RAM test 2".to_string()));
    // In bank 0

    breakpoints.add(true, 0x1f51, Action::Log("Test result check".to_string()));
    breakpoints.add(true, 0x1f51, Action::TraceRegisters);
    breakpoints.add(true, 0x6ad9, Action::Log("Testing completed".to_string()));
    // breakpoints.add(true, 0x6ad9, Action::SetTraceInstructions(true));
    // Force tests to pass
    // breakpoints.add(true, 0x6ad9, Action::Set(Register::PC, 0x6b09));
    // breakpoints.add(true, 0x94ee, Action::Set(Register::B, 0));
    // breakpoints.add(true, 0x94ee, Action::Set(Register::RAM(0x1f), 0));

    breakpoints.add(true, 0xcdf2, Action::Log("Testing DUART".to_string()));

    breakpoints.add(
        true,
        0x2d5e,
        Action::Log("Processing SSU probe".to_string()),
    );

    breakpoints.add(
        true,
        0x16a0d,
        Action::Log("Dispatching keystroke".to_string()),
    );
    breakpoints.add(true, 0x16a0d, Action::TraceRegisters);

    // Jump to setup (careful w/PSW)
    // breakpoints.add(true, 0x169e0, Action::Set(Register::PSW, 0));
    // breakpoints.add(true, 0x169e0, Action::Set(Register::PC, 0x6ac3));
    // breakpoints.add(true, 0x3de9, Action::Set(Register::PC, 0x3df0));
    // breakpoints.add(true, 0x3de9, Action::Set(Register::A, 0));
    // breakpoints.add(true, 0x3de9, Action::Set(Register::R(3), 0));
    // breakpoints.add(true, 0x3df6, Action::Set(Register::A, 0xf2));

    breakpoints.add(true, 0x5521, Action::Log("Loading init string".to_string()));
    breakpoints.add(true, 0x5521, Action::TraceRegisters);

    breakpoints.add(true, 0x15bd6, Action::Log("Video RAM test".to_string()));
    breakpoints.add(
        true,
        0x15c11,
        Action::Log("Video RAM test 1/even".to_string()),
    );
    breakpoints.add(
        true,
        0x15c24,
        Action::Log("Video RAM test 1/odd".to_string()),
    );
    breakpoints.add(
        true,
        0x15c48,
        Action::Log("Video RAM test 2/even".to_string()),
    );
    breakpoints.add(
        true,
        0x15c36,
        Action::Log("Video RAM test 2/odd".to_string()),
    );
    breakpoints.add(
        true,
        0x15c0c,
        Action::Log("Video RAM test failed".to_string()),
    );

    breakpoints.add(
        true,
        0x15ee4,
        Action::Log("Video RAM checkerboard".to_string()),
    );

    breakpoints.add(
        true,
        0x15f81,
        Action::Log("Video latch test outer".to_string()),
    );
    breakpoints.add(true, 0x16074, Action::Log("Video latch test".to_string()));
    breakpoints.add(
        true,
        0x160ba,
        Action::Log("Video latch test 1 failed".to_string()),
    );
    breakpoints.add(true, 0x160ba, Action::TraceRegisters);
    breakpoints.add(
        true,
        0x160f9,
        Action::Log("Video latch test 2 failed".to_string()),
    );
    breakpoints.add(true, 0x160f9, Action::TraceRegisters);

    breakpoints.add(true, 0x160c6, Action::Log("Video latch test 3".to_string()));

    breakpoints.add(
        true,
        0x15c0c,
        Action::Log("Video RAM test failed".to_string()),
    );
    breakpoints.add(true, 0x15c0c, Action::TraceRegisters);
    breakpoints.add(
        true,
        0x15c59,
        Action::Log("Video RAM test passed".to_string()),
    );

    breakpoints.add(
        true,
        0x15cca,
        Action::Log("Wait for VSYNC (bank 1)".to_string()),
    );
    // breakpoints.add(
    //     true,
    //     0x15d11,
    //     Action::Log("Wait for VSYNC #2 (bank 1)".to_string()),
    // );
    breakpoints.add(
        true,
        0x15cd4,
        Action::Log("Wait for VSYNC failed (bank 1)".to_string()),
    );
    breakpoints.add(
        true,
        0x15d26,
        Action::Log("Wait for VSYNC complete (bank 1)".to_string()),
    );

    breakpoints.add(true, 0x15c89, Action::Log("Check VSYNC timing".to_string()));
    breakpoints.add(
        true,
        0x15cc5,
        Action::Log("Check VSYNC timing (failed)".to_string()),
    );

    breakpoints.add(
        true,
        0x2074,
        Action::Log("Wait for VSYNC (bank 0)".to_string()),
    );

    breakpoints.add(true, 0x16153, Action::Log("Keyboard test".to_string()));
    breakpoints.add(
        true,
        0x16184,
        Action::Log("Keyboard test (failed)".to_string()),
    );
    breakpoints.add(
        true,
        0x1616e,
        Action::Log("Keyboard test (success)".to_string()),
    );

    breakpoints.add(true, 0x5b6d, Action::Log("NVR read".to_string()));
    breakpoints.add(true, 0x5b5e, Action::Log("NVR read checksum".to_string()));
    breakpoints.add(true, 0x5b5e, Action::TraceRegisters);
    breakpoints.add(true, 0x5b90, Action::Log("NVR write".to_string()));
    breakpoints.add(true, 0x5b90, Action::TraceRegisters);

    breakpoints.add(true, 0x5c60, Action::Log("NVR fail 1".to_string()));
    breakpoints.add(true, 0x5cba, Action::Log("NVR fail 2".to_string()));
    breakpoints.add(true, 0x5ab3, Action::Log("NVR fail 3".to_string()));
    breakpoints.add(true, 0x5a59, Action::Log("NVR fail 4".to_string()));

    info!("CPU initialized, PC = 0x{:04X}", cpu.pc_ext(&system));

    if args.debug {
        use i8051_debug_tui::{Debugger, DebuggerState, crossterm};
        let mut debugger = Debugger::new(Default::default())?;
        tracing_subscriber::registry()
            .with(debugger.tracing_collector())
            .init();
        debugger.enter()?;
        for breakpoint in args.breakpoint {
            debugger.breakpoints_mut().insert(breakpoint);
        }
        let mut instruction_count = 0_usize;
        loop {
            match debugger.debugger_state() {
                DebuggerState::Quit => {
                    debugger.exit()?;
                    break;
                }
                DebuggerState::Paused => {
                    debugger.render(&cpu, &mut system)?;
                    let event = crossterm::event::poll(Duration::from_millis(100))?;
                    if event {
                        let event = crossterm::event::read()?;
                        if debugger.handle_event(event, &mut cpu, &mut system) {
                            system.step(&mut cpu);
                        }
                    }
                }
                DebuggerState::Running => {
                    instruction_count += 1;
                    if instruction_count % 0x10000 == 0 {
                        debugger.render(&cpu, &mut system)?;
                        let event = crossterm::event::poll(Duration::from_millis(0))?;
                        if event {
                            let event = crossterm::event::read()?;
                            if debugger.handle_event(event, &mut cpu, &mut system) {
                                system.step(&mut cpu);
                                debugger.render(&cpu, &mut system)?;
                            }
                        }
                    }
                    system.step(&mut cpu);
                    if debugger.breakpoints().contains(&cpu.pc_ext(&system)) {
                        debugger.pause();
                    }
                }
            }
        }
    } else {
        let mut terminal = ratatui::Terminal::new(CrosstermBackend::new(stdout()))?;

        if args.display {
            crossterm::terminal::enable_raw_mode()?;
            crossterm::execute!(io::stdout(), crossterm::terminal::EnterAlternateScreen,)?;
            terminal.clear()?;
        }

        // CPU execution loop
        let mut running = true;
        let mut pc_trace = false;
        let mut compose_special_key = false;
        let mut hex = DisplayMode::Normal;
        loop {
            if running {
                let pc = cpu.pc_ext(&system);
                system.step(&mut cpu);

                let new_pc = cpu.pc_ext(&system);
                if new_pc & 0xffff == 0 {
                    warn!("CPU reset detected at PC = 0x{:04X}", pc);
                }
                if (0xbb..0x110).contains(&new_pc) {
                    warn!(
                        "CPU weird step ({:02X}) detected at PC = 0x{:04X}",
                        new_pc, pc
                    );
                }

                instruction_count += 1;
            }

            if args.display && (instruction_count % 0x1000 == 0 || !running) {
                if crossterm::event::poll(Duration::from_millis(0))? {
                    let start = Instant::now();
                    let event = crossterm::event::read()?;
                    if start.elapsed() > Duration::from_millis(100) {
                        warn!("Event read took too long: {:?}", start.elapsed());
                    }
                    if let Event::Key(key) = event {
                        let sender = system.keyboard.sender();

                        if compose_special_key {
                            compose_special_key = false;
                            if key.modifiers.is_empty() {
                                match key.code {
                                    KeyCode::Char('1') => {
                                        _ = sender.send_special_key(SpecialKey::F1);
                                    }
                                    KeyCode::Char('2') => {
                                        _ = sender.send_special_key(SpecialKey::F2);
                                    }
                                    KeyCode::Char('3') => {
                                        _ = sender.send_special_key(SpecialKey::F3);
                                    }
                                    KeyCode::Char('4') => {
                                        _ = sender.send_special_key(SpecialKey::F4);
                                    }
                                    KeyCode::Char('5') => {
                                        _ = sender.send_special_key(SpecialKey::F5);
                                    }
                                    KeyCode::Char('c') => {
                                        _ = sender.send_special_key(SpecialKey::Lock);
                                    }
                                    KeyCode::Char('q') => {
                                        crossterm::terminal::disable_raw_mode()?;
                                        crossterm::execute!(
                                            io::stdout(),
                                            crossterm::terminal::LeaveAlternateScreen,
                                        )?;
                                        break;
                                    }
                                    _ => {}
                                }
                            }
                        }
                        if key.modifiers == KeyModifiers::CONTROL {
                            match key.code {
                                KeyCode::Char('q') => {
                                    crossterm::terminal::disable_raw_mode()?;
                                    crossterm::execute!(
                                        io::stdout(),
                                        crossterm::terminal::LeaveAlternateScreen,
                                    )?;
                                    break;
                                }
                                KeyCode::Char('f') => {
                                    compose_special_key = true;
                                }
                                KeyCode::Char(' ') => {
                                    running = !running;
                                }
                                KeyCode::Char('h') => {
                                    hex = match hex {
                                        DisplayMode::Normal => DisplayMode::NibbleTriplet,
                                        DisplayMode::NibbleTriplet => DisplayMode::Bytes,
                                        DisplayMode::Bytes => DisplayMode::Normal,
                                    };
                                }
                                KeyCode::Char('d') => {
                                    fs::write("/tmp/vram.bin", &system.memory.vram[0..])?;
                                }
                                #[cfg(feature = "pc-trace")]
                                KeyCode::Char('p') => {
                                    use std::io::Write;
                                    if !pc_trace {
                                        system.pc_bitset_current = system.pc_bitset.clone();
                                        pc_trace = true;
                                        let mut pc_trace_file = File::create("/tmp/pc_trace.txt")?;
                                        writeln!(pc_trace_file, "PC trace started")?;
                                    } else {
                                        let difference =
                                            system.pc_bitset.difference(&system.pc_bitset_current);
                                        let mut pc_trace_file = File::create("/tmp/pc_trace.txt")?;
                                        for pc in difference {
                                            writeln!(pc_trace_file, "0x{:04X}", pc)?;
                                        }
                                        pc_trace = false;
                                    }
                                }
                                KeyCode::Char(c) => {
                                    _ = sender.send_ctrl_char(c);
                                }
                                KeyCode::F(1) => {
                                    _ = sender.send_ctrl_special_key(SpecialKey::F1);
                                }
                                KeyCode::F(2) => {
                                    _ = sender.send_ctrl_special_key(SpecialKey::F2);
                                }
                                KeyCode::F(3) => {
                                    _ = sender.send_ctrl_special_key(SpecialKey::F3);
                                }
                                KeyCode::F(4) => {
                                    _ = sender.send_ctrl_special_key(SpecialKey::F4);
                                }
                                KeyCode::F(5) => {
                                    _ = sender.send_ctrl_special_key(SpecialKey::F5);
                                }
                                KeyCode::Up => {
                                    _ = sender.send_ctrl_special_key(SpecialKey::Up);
                                }
                                KeyCode::Down => {
                                    _ = sender.send_ctrl_special_key(SpecialKey::Down);
                                }
                                KeyCode::Left => {
                                    _ = sender.send_ctrl_special_key(SpecialKey::Left);
                                }
                                KeyCode::Right => {
                                    _ = sender.send_ctrl_special_key(SpecialKey::Right);
                                }
                                _ => {}
                            }
                        }
                        if key.modifiers == KeyModifiers::SHIFT | KeyModifiers::CONTROL {
                            match key.code {
                                KeyCode::Up => {
                                    _ = sender.send_shift_ctrl_special_key(SpecialKey::Up);
                                }
                                KeyCode::Down => {
                                    _ = sender.send_shift_ctrl_special_key(SpecialKey::Down);
                                }
                                KeyCode::Left => {
                                    _ = sender.send_shift_ctrl_special_key(SpecialKey::Left);
                                }
                                KeyCode::Right => {
                                    _ = sender.send_shift_ctrl_special_key(SpecialKey::Right);
                                }
                                _ => {}
                            }
                        }
                        if key.modifiers == KeyModifiers::SHIFT {
                            match key.code {
                                KeyCode::Char(c) => {
                                    _ = sender.send_char(c);
                                }
                                KeyCode::Up => {
                                    _ = sender.send_shift_special_key(SpecialKey::Up);
                                }
                                KeyCode::Down => {
                                    _ = sender.send_shift_special_key(SpecialKey::Down);
                                }
                                KeyCode::Left => {
                                    _ = sender.send_shift_special_key(SpecialKey::Left);
                                }
                                KeyCode::Right => {
                                    _ = sender.send_shift_special_key(SpecialKey::Right);
                                }
                                _ => {}
                            }
                        }
                        if key.modifiers.is_empty() {
                            match key.code {
                                KeyCode::Char(c) => {
                                    _ = sender.send_char(c);
                                }
                                KeyCode::Left => {
                                    _ = sender.send_special_key(SpecialKey::Left);
                                }
                                KeyCode::Right => {
                                    _ = sender.send_special_key(SpecialKey::Right);
                                }
                                KeyCode::Up => {
                                    _ = sender.send_special_key(SpecialKey::Up);
                                }
                                KeyCode::Down => {
                                    _ = sender.send_special_key(SpecialKey::Down);
                                }
                                KeyCode::Backspace => {
                                    _ = sender.send_special_key(SpecialKey::Delete);
                                }
                                KeyCode::Enter => {
                                    _ = sender.send_special_key(SpecialKey::Return);
                                }
                                KeyCode::Esc => {
                                    sender.send_escape();
                                }

                                KeyCode::F(1) => {
                                    _ = sender.send_special_key(SpecialKey::F1);
                                }
                                KeyCode::F(2) => {
                                    _ = sender.send_special_key(SpecialKey::F2);
                                }
                                KeyCode::F(3) => {
                                    _ = sender.send_special_key(SpecialKey::F3);
                                }
                                KeyCode::F(4) => {
                                    _ = sender.send_special_key(SpecialKey::F4);
                                }
                                KeyCode::F(5) => {
                                    _ = sender.send_special_key(SpecialKey::F5);
                                }
                                _ => {}
                            }
                        }
                    }
                }

                let vram = &system.memory.vram[0..];
                // Skip redrawing if the chargen is disabled
                if system.memory.mapper.get(6) & 0xf0 != 0xf0 {
                    terminal.draw(|f| {
                        let screen = Screen::new(vram, &system.memory.mapper).display_mode(hex);
                        f.render_widget(screen, f.area());
                        let stage = Span::styled(
                            format!(
                                "{:b}/{:02X}",
                                cpu.internal_ram[0x1f], cpu.internal_ram[0x7e]
                            ),
                            Style::default().fg(Color::LightBlue),
                        );
                        let stage = stage.into_right_aligned_line();
                        f.render_widget(stage, f.area());

                        if args.show_mapper {
                            let mut mapper_line = Line::default();
                            for i in 0..16 {
                                let attr = system.memory.mapper.get(i);
                                let style = Style::default().fg(Color::Indexed(attr));
                                let text = if i == 6 || i == 9 || i == 10 || i == 11 || i == 12 {
                                    Span::styled(
                                        format!(
                                            "{:02X}/{:02X} ",
                                            system.memory.mapper.get(i),
                                            system.memory.mapper.get2(i)
                                        ),
                                        style,
                                    )
                                } else {
                                    Span::styled(
                                        format!("{:02X} ", system.memory.mapper.get(i)),
                                        style,
                                    )
                                };
                                mapper_line.push_span(text);
                            }
                            mapper_line.push_span(format!(
                                "{:02X} {:02X} {:02X}",
                                cpu.sfr(SFR_P1, &system),
                                cpu.sfr(SFR_P2, &system),
                                cpu.sfr(SFR_P3, &system)
                            ));
                            f.render_widget(mapper_line, f.area());
                        }

                        if args.show_vram {
                            let vram = &system.memory.vram[0..256];
                            for i in 0..8 {
                                let mut vram_line = Line::default();
                                for j in 0..32 {
                                    let attr = vram[i * 32 + j];
                                    let style = Style::default().fg(Color::Indexed(attr));
                                    let text = Span::styled(format!("{:02X} ", attr), style);
                                    vram_line.push_span(text);
                                }
                                f.render_widget(
                                    vram_line,
                                    f.area().offset(Offset {
                                        x: 0,
                                        y: (f.area().height as i32 - 8) + i as i32,
                                    }),
                                );
                            }
                        }
                    })?;
                }
            }
        }
    }

    let elapsed = start_time.elapsed();
    info!("CPU execution completed:");
    info!("  Instructions executed: {}", instruction_count);
    info!("  Final PC: 0x{:04X}", cpu.pc);
    info!("  Time elapsed: {:?}", elapsed);
    if elapsed.as_secs_f64() > 0.0 {
        info!(
            "  Instructions per second: {:.0}",
            instruction_count as f64 / elapsed.as_secs_f64()
        );
    }

    info!("VT420 emulator execution completed!");

    Ok(())
}
