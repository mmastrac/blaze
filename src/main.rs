use clap::Parser;
use i8051::breakpoint::{Action, Breakpoints};
use i8051::peripheral::{Serial, Timer};
use ratatui::crossterm::event::{Event, KeyCode};
use ratatui::style::{Color, Style};
use ratatui::text::Span;
use std::io::{self, IsTerminal, stdout};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tracing::{Level, info, trace, warn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm;

mod lk201;
mod memory;
mod screen;
mod video;

use memory::{Bank, RAM, ROM, VideoProcessor};

use i8051::{Cpu, DefaultPortMapper, Register};

use crate::lk201::LK201;
use crate::memory::DiagnosticMonitor;
use crate::screen::Screen;

/// VT420 Terminal Emulator
/// Emulates a VT420 terminal using an 8051 microcontroller
#[derive(Parser)]
#[command(name = "vt-emulator")]
#[command(about = "A VT420 terminal emulator using 8051 CPU emulation")]
struct Args {
    /// Path to the ROM file
    #[arg(long)]
    rom: PathBuf,

    /// Display the video output
    #[arg(long)]
    display: bool,

    /// Enable debugger
    #[arg(long)]
    debug: bool,

    /// Breakpoints for debug mode, repeatable, parsed as hex
    #[arg(long, value_parser = parse_hex_address)]
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

    if args.debug || args.display {
        // ?
    } else {
        let level = if args.verbose {
            Level::TRACE
        } else {
            Level::INFO
        };
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
    let bank = Bank::default();

    // Load ROM into memory
    info!("Loading ROM into memory...");
    let mut code = ROM::new(&args.rom, bank.bank.clone())?;

    info!("ROM loaded: {} bytes", code.rom_size());
    info!(
        "ROM banks: {} ({} bytes each)",
        code.num_banks(),
        code.bank_size()
    );
    info!(
        "Current ROM bank: {} ({} 64KB)",
        code.rom_bank(),
        if code.rom_bank() == 0 {
            "first"
        } else {
            "remaining"
        }
    );

    // Initialize the 8051 CPU emulator
    info!("Initializing 8051 CPU emulator...");
    let mut cpu = Cpu::new();

    // Start CPU execution
    info!("Starting CPU execution...");
    let start_time = Instant::now();
    let mut instruction_count = 0;

    let video_row = VideoProcessor::new();
    let mut ram = RAM::new(bank.bank.clone(), video_row.sync.clone());
    let (mut serial, in_kbd, out_kbd) = Serial::new(60);
    let mut default = DefaultPortMapper::default();

    let diagnostic_monitor = DiagnosticMonitor::default();
    let timer = Timer::default();
    let mut ports = (
        bank,
        (video_row, (serial, (diagnostic_monitor, (timer, default)))),
    );

    let mut breakpoints = Breakpoints::new();
    let keyboard_pipe = in_kbd.clone();
    let mut keyboard = LK201::new(in_kbd, out_kbd);

    // Enable tracing if requested
    // if args.trace && args.verbose {
    //     info!("Instruction tracing enabled");
    //     breakpoints.add(true, 0, Action::SetTraceInstructions(true));
    //     breakpoints.add(true, 0x10000, Action::SetTraceInstructions(true));
    // }

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
        Action::Log("Interrupt:Entering user code".to_string()),
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
        0x5ed1,
        Action::Log("Testing keyboard serial".to_string()),
    );

    breakpoints.add(true, 0xf0d, Action::Log("Update DUART bits".to_string()));
    breakpoints.add(true, 0x10f0d, Action::Log("Update DUART bits".to_string()));

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
    breakpoints.add(true, 0x94ee, Action::Set(Register::B, 0));
    breakpoints.add(true, 0x94ee, Action::Set(Register::RAM(0x1f), 0));

    // Skip AD22 loop(s)
    // breakpoints.add(true, 0x950d, Action::Set(Register::PC, 0x951e));
    // breakpoints.add(true, 0x9513, Action::Set("PC".to_string(), 0x9516)); //works
    // breakpoints.add(true, 0x9513, Action::Set("PC".to_string(), 0x951e));
    // breakpoints.add(true, 0x957c, Action::Set("PC".to_string(), 0x9581));
    // breakpoints.add(true, 0x3bae, Action::Set("PC".to_string(), 0x3bcb));
    // let mut once = std::sync::Once::new();
    // breakpoints.add(
    //     true,
    //     0x950a,
    //     Action::Run(Box::new(move |cpu| {
    //         once.call_once(|| {
    //             trace!("Sending F3...");
    //             // _ = keyboard_pipe.send(0x58);
    //             // _ = keyboard_pipe.send(0x58);
    //             // _ = keyboard_pipe.send(0xB4);
    //             // let byte_24 = cpu.internal_ram(0x24);
    //             // cpu.internal_ram_write(0x24, byte_24 | 1);
    //             // cpu.internal_ram_write(0x24, byte_24 | 2);
    //         });
    //     })),
    // );

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

    // Skip EEPROM read
    // breakpoints.add(true, 0x1143, Action::SetTraceInstructions(true));
    breakpoints.add(true, 0x5b6d, Action::Set(Register::PC, 0x5b8d));

    let mut context = (ports, ram, code);
    info!("CPU initialized, PC = 0x{:04X}", cpu.pc_ext(&context));

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
                    debugger.render(&cpu, &mut context)?;
                    let event = crossterm::event::poll(Duration::from_millis(100))?;
                    if event {
                        let event = crossterm::event::read()?;
                        if debugger.handle_event(event, &mut cpu, &mut context) {
                            breakpoints.run(true, &mut cpu, &mut context);
                            cpu.step(&mut context);
                            keyboard.tick();
                            context.0.1.1.0.tick(&mut cpu);
                            context.0.1.0.tick();
                            let tick = context.0.1.1.1.1.0.prepare_tick(&mut cpu, &context);
                            context.0.1.1.1.1.0.tick(&mut cpu, tick);
                            breakpoints.run(false, &mut cpu, &mut context);
                            debugger.render(&cpu, &mut context)?;
                        }
                    }
                }
                DebuggerState::Running => {
                    instruction_count += 1;
                    if instruction_count % 0x10000 == 0 {
                        debugger.render(&cpu, &mut context)?;
                        let event = crossterm::event::poll(Duration::from_millis(0))?;
                        if event {
                            let event = crossterm::event::read()?;
                            if debugger.handle_event(event, &mut cpu, &mut context) {
                                cpu.step(&mut context);
                                debugger.render(&cpu, &mut context)?;
                            }
                        }
                    }
                    breakpoints.run(true, &mut cpu, &mut context);
                    cpu.step(&mut context);
                    keyboard.tick();
                    context.0.1.1.0.tick(&mut cpu);
                    context.0.1.0.tick();
                    let tick = context.0.1.1.1.1.0.prepare_tick(&mut cpu, &context);
                    context.0.1.1.1.1.0.tick(&mut cpu, tick);
                    breakpoints.run(false, &mut cpu, &mut context);
                    if debugger.breakpoints().contains(&cpu.pc_ext(&context)) {
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
        let mut hex = false;
        loop {
            if running {
                let pc = cpu.pc_ext(&context);
                breakpoints.run(true, &mut cpu, &mut context);
                cpu.step(&mut context);
                breakpoints.run(false, &mut cpu, &mut context);
                keyboard.tick();
                context.0.1.1.0.tick(&mut cpu);
                context.0.1.0.tick();
                let tick = context.0.1.1.1.1.0.prepare_tick(&mut cpu, &context);
                context.0.1.1.1.1.0.tick(&mut cpu, tick);

                let new_pc = cpu.pc_ext(&context);
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

            if args.display && (instruction_count % 0x100 == 0 || !running) {
                if crossterm::event::poll(Duration::from_millis(0))? {
                    let event = crossterm::event::read()?;
                    if let Event::Key(key) = event
                        && key.modifiers.is_empty()
                    {
                        match key.code {
                            KeyCode::Char('q') => {
                                crossterm::terminal::disable_raw_mode()?;
                                crossterm::execute!(
                                    io::stdout(),
                                    crossterm::terminal::LeaveAlternateScreen,
                                )?;
                                break;
                            }
                            KeyCode::Char(' ') => {
                                running = !running;
                            }
                            KeyCode::Char('h') => {
                                hex = !hex;
                            }
                            KeyCode::Left => {
                                _ = keyboard_pipe.send(0xa7);
                            }
                            KeyCode::Right => {
                                _ = keyboard_pipe.send(0xa8);
                            }
                            KeyCode::Up => {
                                _ = keyboard_pipe.send(0xaa);
                            }
                            KeyCode::Down => {
                                _ = keyboard_pipe.send(0xa9);
                            }
                            KeyCode::Enter => {
                                _ = keyboard_pipe.send(0xbd);
                            }
                            KeyCode::F(3) => {
                                _ = keyboard_pipe.send(0x58);
                            }
                            _ => {}
                        }
                    }
                }
                let vram = &context.1.vram[0..];
                terminal.draw(|f| {
                    let screen = Screen::new(vram).hex_mode(hex);
                    f.render_widget(screen, f.area());
                    let stage = Span::styled(
                        format!("{}", cpu.internal_ram[0x7e]),
                        Style::default().fg(Color::LightBlue),
                    );
                    let stage = stage.into_right_aligned_line();
                    f.render_widget(stage, f.area());
                })?;
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
