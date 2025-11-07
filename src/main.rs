use clap::Parser;
use i8051::breakpoint::Action;
use i8051::sfr::{SFR_P1, SFR_P2, SFR_P3};
use ratatui::crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::layout::Offset;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use std::fs::{self, File};
use std::io::{self, IsTerminal, stdout};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tracing::{Level, info, warn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm;

mod host;
mod machine;

use machine::vt420::System;

use i8051::Cpu;

use crate::host::comm::CommConfig;
use crate::host::screen::{DisplayMode, Screen};
use crate::machine::generic::lk201::SpecialKey;

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
