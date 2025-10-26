use clap::Parser;
use i8051::breakpoint::{Action, Breakpoints};
use i8051::peripheral::Serial;
use std::collections::VecDeque;
use std::io::{IsTerminal, stdout};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tracing::{Level, info, trace};

mod memory;

use memory::{Bank, RAM, ROM, VideoRow};

use i8051::{Cpu, DefaultPortMapper, sfr::*};

/// VT420 Terminal Emulator
/// Emulates a VT420 terminal using an 8051 microcontroller
#[derive(Parser)]
#[command(name = "vt-emulator")]
#[command(about = "A VT420 terminal emulator using 8051 CPU emulation")]
struct Args {
    /// Path to the ROM file
    #[arg(short, long)]
    rom: PathBuf,

    /// Enable debugger
    #[arg(short, long)]
    debug: bool,

    /// Enable instruction tracing
    #[arg(short, long)]
    trace: bool,

    /// Enable verbose output
    #[arg(short, long)]
    verbose: bool,

    /// ROM bank to use (0 = lower half, 1 = upper half)
    #[arg(short, long, default_value = "0")]
    bank: u8,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    if args.debug {
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
                .init();
        } else {
            tracing_subscriber::fmt()
                .with_ansi(false)
                .with_max_level(level)
                .event_format(format)
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

    // Set the requested ROM bank
    if args.bank <= 1 {
        code.set_rom_bank(args.bank);
    } else {
        info!("Error: Bank must be 0 or 1, got {}", args.bank);
        std::process::exit(1);
    }

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

    let mut ram = RAM::new(bank.bank.clone());
    let (mut serial, in_kbd, out_kbd) = Serial::new();
    let mut kbd_queue = VecDeque::new();
    let mut default = DefaultPortMapper::default();

    let video_row = VideoRow::default();
    let mut ports = (bank, (video_row, (serial, default)));

    let mut breakpoints = Breakpoints::new();

    // Enable tracing if requested
    if args.trace {
        info!("Instruction tracing enabled");
        breakpoints.add(true, 0, Action::SetTraceInstructions(true));
        breakpoints.add(true, 0x10000, Action::SetTraceInstructions(true));
    }

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
    breakpoints.add(true, 0x156, Action::Log("Testing ROM Bank 0".to_string()));
    breakpoints.add(true, 0x15aeb, Action::Log("Testing phase 2".to_string()));
    breakpoints.add(true, 0x15b23, Action::Log("RAM test".to_string()));
    breakpoints.add(true, 0x15b8a, Action::Log("RAM test 2".to_string()));
    // In bank 0

    breakpoints.add(true, 0x1f51, Action::Log("Test result check".to_string()));
    breakpoints.add(true, 0x1f51, Action::TraceRegisters);
    breakpoints.add(true, 0x1f51, Action::Set("B".to_string(), 0));
    breakpoints.add(true, 0x6ad9, Action::Log("Testing completed".to_string()));
    // breakpoints.add(true, 0x6ad9, Action::SetTraceInstructions(true));
    // Force tests to pass
    breakpoints.add(true, 0x6ad9, Action::Set("PC".to_string(), 0x6b09));

    // Skip AD22 loop
    // breakpoints.add(true, 0x950d, Action::Set("PC".to_string(), 0x951e));
    breakpoints.add(true, 0x9513, Action::Set("PC".to_string(), 0x9516));

    // breakpoints.add(true, 0x5fdb, Action::SetTraceInstructions(true));
    // breakpoints.add(true, 0x5fdb, Action::SetTraceRegisters(true));
    // breakpoints.add(true, 0x5f9e, Action::SetTraceInstructions(false));
    // breakpoints.add(true, 0x5f9e, Action::SetTraceRegisters(false));
    breakpoints.add(true, 0x5fe7, Action::TraceRegisters);

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

    breakpoints.add(
        true,
        0x160c6,
        Action::Log("Video latch test 3 (skip)".to_string()),
    );
    breakpoints.add(true, 0x160c6, Action::Set("PC".to_string(), 0x6118));

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
    breakpoints.add(true, 0x15cca, Action::Set("PC".to_string(), 0x5d28));

    breakpoints.add(
        true,
        0x15c89,
        Action::Log("Wait for VSYNC line (bank 1)".to_string()),
    );
    breakpoints.add(true, 0x15c89, Action::Set("PC".to_string(), 0x5cc4));

    breakpoints.add(
        true,
        0x2074,
        Action::Log("Wait for VSYNC (bank 0)".to_string()),
    );
    breakpoints.add(true, 0x2074, Action::Set("PC".to_string(), 0x20c9));

    breakpoints.add(true, 0x16153, Action::Log("Keyboard test".to_string()));

    // breakpoints.add(true, 0x56c2, Action::SetTraceInstructions(true));
    // breakpoints.add(true, 0x56c2, Action::SetTraceRegisters(true));

    // breakpoints.add(true, 0x57F8, Action::Set("DPTR".to_string(), 0x5a4a));
    // breakpoints.add(true, 0x5c3d, Action::Log("Waiting for EEPROM".to_string()));

    breakpoints.add(true, 0x1C1B, Action::TraceRegisters);

    // breakpoints.add(true, 0x7be2, Action::SetTraceInstructions(true));
    // breakpoints.add(true, 0x7be2, Action::SetTraceRegisters(true));

    // Skip the KBD interrupt check: JB 21.3,LAB_RAM_001006
    breakpoints.add(true, 0x11006, Action::Set("PC".to_string(), 0x1009));

    // Skip EEPROM read
    // breakpoints.add(true, 0x1143, Action::SetTraceInstructions(true));
    breakpoints.add(true, 0x5b6d, Action::Set("PC".to_string(), 0x5b8d));

    let mut context = (ports, ram, code);
    info!("CPU initialized, PC = 0x{:04X}", cpu.pc_ext(&context));

    cpu.sfr_set(SFR_P3, 1 << 3, &mut context);

    if args.debug {
        use i8051_debug_tui::{Debugger, DebuggerState, crossterm};
        let mut debugger = Debugger::new(Default::default())?;
        debugger.enter()?;
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
                            cpu.step(&mut context);
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
                    cpu.step(&mut context);
                    if debugger.breakpoints().contains(&cpu.pc_ext(&context)) {
                        debugger.pause();
                    }
                    breakpoints.run(true, &mut cpu, &mut context);
                }
            }
        }
    } else {
        // CPU execution loop
        loop {
            if let Ok(value) = out_kbd.try_recv() {
                // trace!("KBD: {:02X}", value);
                kbd_queue.push_back(value);

                match kbd_queue.front().unwrap() {
                    // Ping?
                    0x55 => {
                        trace!("KBD: Ping");
                        kbd_queue.pop_front();
                        _ = in_kbd.send(1);
                        _ = in_kbd.send(0);
                        _ = in_kbd.send(0);
                        _ = in_kbd.send(0);
                    }
                    0x11 | 0x13 => {
                        if kbd_queue.len() >= 2 {
                            trace!("KBD: LED state");
                            kbd_queue.pop_front();
                            kbd_queue.pop_front();
                        }
                    }
                    0x99 => {
                        trace!("KBD: disable click");
                        kbd_queue.pop_front();
                        _ = in_kbd.send(0xba);
                    }
                    0xAB => {
                        trace!("KBD: Request ID");
                        kbd_queue.pop_front();
                        _ = in_kbd.send(1);
                        _ = in_kbd.send(0);
                    }
                    0xE1 => {
                        trace!("KBD: Disable repeat");
                        kbd_queue.pop_front();
                        _ = in_kbd.send(0xba);
                    }
                    0xE3 => {
                        trace!("KBD: Enable repeat");
                        kbd_queue.pop_front();
                        _ = in_kbd.send(0xba);
                    }
                    0xFD => {
                        trace!("KBD: Power-up");
                        kbd_queue.pop_front();
                        _ = in_kbd.send(1);
                        _ = in_kbd.send(0);
                        _ = in_kbd.send(0);
                        _ = in_kbd.send(0);
                    }
                    _ => {
                        trace!("KBD (unknown): {:02X}", value);
                        kbd_queue.clear();
                        _ = in_kbd.send(0xB6);
                    }
                }
            }
            breakpoints.run(true, &mut cpu, &mut context);
            // trace!("PC = 0x{:04X}", cpu.pc);
            cpu.step(&mut context);
            breakpoints.run(false, &mut cpu, &mut context);

            instruction_count += 1;
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
