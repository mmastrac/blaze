use clap::Parser;
use i8051::breakpoint::{Action, Breakpoints};
use i8051::peripheral::Serial;
use std::cell::Cell;
use std::collections::VecDeque;
use std::time::Instant;
use std::{path::PathBuf, rc::Rc};

mod memory;

use memory::{RAM, ROM};

use i8051::{Cpu, DefaultPortMapper, PortMapper, sfr::*};

use crate::memory::VideoRow;

/// VT420 Terminal Emulator
/// Emulates a VT420 terminal using an 8051 microcontroller
#[derive(Parser)]
#[command(name = "vt-emulator")]
#[command(about = "A VT420 terminal emulator using 8051 CPU emulation")]
struct Args {
    /// Path to the ROM file
    #[arg(short, long)]
    rom: PathBuf,

    /// Maximum number of instructions to execute (0 = unlimited)
    #[arg(short, long, default_value = "1000000")]
    max_instructions: u64,

    /// Enable debug output
    #[arg(short, long)]
    debug: bool,

    /// Enable instruction tracing
    #[arg(short, long)]
    trace: bool,

    /// ROM bank to use (0 = lower half, 1 = upper half)
    #[arg(short, long, default_value = "1")]
    bank: u8,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    println!("VT420 Emulator starting...");
    println!("ROM file: {:?}", args.rom);

    // Check if ROM file exists
    if !args.rom.exists() {
        eprintln!("Error: ROM file does not exist: {:?}", args.rom);
        std::process::exit(1);
    }

    let rom_bank = Rc::new(Cell::new(args.bank == 1));

    // Load ROM into memory
    println!("Loading ROM into memory...");
    let mut code = ROM::new(&args.rom, rom_bank.clone())?;

    // Set the requested ROM bank
    if args.bank <= 1 {
        code.set_rom_bank(args.bank);
    } else {
        eprintln!("Error: Bank must be 0 or 1, got {}", args.bank);
        std::process::exit(1);
    }

    println!("ROM loaded: {} bytes", code.rom_size());
    println!(
        "ROM banks: {} ({} bytes each)",
        code.num_banks(),
        code.bank_size()
    );
    println!(
        "Current ROM bank: {} ({} 64KB)",
        code.rom_bank(),
        if code.rom_bank() == 0 {
            "first"
        } else {
            "remaining"
        }
    );

    // Show the first 32 bytes of the current ROM bank
    println!(
        "\nFirst 32 bytes of current ROM bank (bank {}):",
        code.rom_bank()
    );
    for i in 0..32.min(code.bank_size()) {
        if i % 16 == 0 {
            print!("{:04X}: ", i);
        }
        print!("{:02X} ", code.read(i as u16));
        if i % 16 == 15 {
            println!();
        }
    }
    if code.bank_size() % 16 != 0 {
        println!();
    }

    // Initialize the 8051 CPU emulator
    println!("\nInitializing 8051 CPU emulator...");
    let mut cpu = Cpu::new();

    // Enable tracing if requested
    if args.trace {
        println!("Instruction tracing enabled");
    }

    println!("CPU initialized, PC = 0x{:04X}", cpu.pc);

    // Start CPU execution
    println!("\nStarting CPU execution...");
    let start_time = Instant::now();
    let mut instruction_count = 0;

    let video_row = VideoRow::new();

    let mut ram = RAM::new(rom_bank.clone(), video_row.clone());
    let (mut serial, in_kbd, out_kbd) = Serial::new();
    let mut kbd_queue = VecDeque::new();
    let mut default = DefaultPortMapper::default();
    default.write(SFR_P3, 1 << 3);

    let mut ports = (serial, (video_row, default));

    let mut breakpoints = Breakpoints::new();

    breakpoints.add(true, 0x0084, Action::Log("Bank Dispatch".to_string()));
    breakpoints.add(true, 0x0084, Action::TraceRegisters);
    breakpoints.add(true, 0x009a, Action::TraceRegisters);

    breakpoints.add(true, 0x5a88, Action::Log("Test failed!!!".to_string()));
    breakpoints.add(true, 0x5d5a, Action::Log("Testing failed!!!".to_string()));

    breakpoints.add(
        true,
        0x5ed1,
        Action::Log("Testing keyboard serial".to_string()),
    );
    breakpoints.add(true, 0xf0d, Action::Log("Testing DUART".to_string()));
    breakpoints.add(true, 0x5ad0, Action::Log("Testing ROM Bank 1".to_string()));
    breakpoints.add(true, 0x156, Action::Log("Testing ROM Bank 0".to_string()));
    breakpoints.add(true, 0x5aeb, Action::Log("Testing phase 2".to_string()));
    breakpoints.add(true, 0x5b23, Action::Log("RAM test".to_string()));
    breakpoints.add(true, 0x5b8a, Action::Log("RAM test 2".to_string()));
    breakpoints.add(true, 0x5bd6, Action::Log("Video RAM test".to_string()));

    // breakpoints.add(true, 0x5fdb, Action::SetTraceInstructions(true));
    // breakpoints.add(true, 0x5fdb, Action::SetTraceRegisters(true));
    // breakpoints.add(true, 0x5f9e, Action::SetTraceInstructions(false));
    // breakpoints.add(true, 0x5f9e, Action::SetTraceRegisters(false));
    breakpoints.add(true, 0x5fe7, Action::TraceRegisters);

    breakpoints.add(
        true,
        0x5c11,
        Action::Log("Video RAM test 1/even".to_string()),
    );
    breakpoints.add(
        true,
        0x5c24,
        Action::Log("Video RAM test 1/odd".to_string()),
    );
    breakpoints.add(
        true,
        0x5c48,
        Action::Log("Video RAM test 2/even".to_string()),
    );
    breakpoints.add(
        true,
        0x5c36,
        Action::Log("Video RAM test 2/odd".to_string()),
    );

    breakpoints.add(true, 0x5f81, Action::Log("Video latch test".to_string()));
    breakpoints.add(
        true,
        0x60ba,
        Action::Log("Video latch test 1 failed".to_string()),
    );
    breakpoints.add(true, 0x60ba, Action::TraceRegisters);
    breakpoints.add(
        true,
        0x60f9,
        Action::Log("Video latch test 2 failed".to_string()),
    );
    breakpoints.add(true, 0x60f9, Action::TraceRegisters);

    breakpoints.add(true, 0x60c6, Action::Log("Video latch test 3".to_string()));
    breakpoints.add(true, 0x60c6, Action::Set("PC".to_string(), 0x6118));

    breakpoints.add(
        true,
        0x5c0c,
        Action::Log("Video RAM test failed".to_string()),
    );
    breakpoints.add(true, 0x5c0c, Action::TraceRegisters);
    breakpoints.add(
        true,
        0x5c59,
        Action::Log("Video RAM test passed".to_string()),
    );

    breakpoints.add(true, 0x5cca, Action::Log("Wait for VSYNC".to_string()));
    breakpoints.add(true, 0x5cca, Action::Set("PC".to_string(), 0x5d28));

    breakpoints.add(true, 0x2074, Action::Log("Wait for VSYNC".to_string()));
    breakpoints.add(true, 0x2074, Action::Set("PC".to_string(), 0x20c9));

    breakpoints.add(true, 0x5c89, Action::Log("Wait for VSYNC line".to_string()));
    breakpoints.add(true, 0x5c89, Action::Set("PC".to_string(), 0x5cc4));

    // breakpoints.add(true, 0x56c2, Action::SetTraceInstructions(true));
    // breakpoints.add(true, 0x56c2, Action::SetTraceRegisters(true));

    breakpoints.add(true, 0x57F8, Action::Set("DPTR".to_string(), 0x5a4a));
    breakpoints.add(true, 0x5c3d, Action::Log("Waiting for EEPROM".to_string()));

    breakpoints.add(true, 0x1C1B, Action::TraceRegisters);

    breakpoints.add(true, 0x5521, Action::TraceInstructions);
    breakpoints.add(true, 0x5521, Action::TraceRegisters);

    // breakpoints.add(true, 0x7be2, Action::SetTraceInstructions(true));
    // breakpoints.add(true, 0x7be2, Action::SetTraceRegisters(true));

    // Skip the KBD interrupt check: JB 21.3,LAB_RAM_001006
    breakpoints.add(true, 0x1006, Action::Set("PC".to_string(), 0x1009));

    // CPU execution loop
    loop {
        if instruction_count >= args.max_instructions {
            println!(
                "Reached maximum instruction limit: {}",
                args.max_instructions
            );
            break;
        }

        let pc = cpu.pc;
        // Execute one instruction

        // if pc == 0x595D {
        //     println!("Skipping loop");
        //     cpu.a_set(1);
        // }
        // if pc == 0x5960 {
        //     println!("Skipping loop");
        //     *cpu.r_mut(0) = 1;
        // }
        // if pc == 0x5e9b {
        //     println!("Skipping loop");
        //     cpu.a_set(1);
        // }
        // if pc == 0x6ADB {
        //     // *cpu.sfr_mut(SFR_A) = 0;
        // }
        // if pc == 0x254d {
        //     println!("Writing char: {:?}", cpu.a() as char);
        // }
        // if pc == 0x6190 || pc == 0x5dB3 {
        //     println!("PORT Give char");
        //     // cpu.sfr_set(SFR_SCON, 1, &mut ports);
        //     // cpu.sfr_set(SFR_SBUF, 0xae, &mut ports);
        // }
        // let op = code.read(pc);
        // if op & 0xF8 == 0xD8 && code.read(pc.wrapping_add(1)) == 0xFE {
        //     if args.trace {
        //         println!("(skipping loop)");
        //     }
        //     *cpu.r_mut(op & 0x07) = 0;
        //     cpu.pc = pc.wrapping_add(2);
        //     continue;
        // }

        ports.tick();
        if let Ok(value) = out_kbd.try_recv() {
            // println!("KBD: {:02X}", value);
            kbd_queue.push_back(value);

            match kbd_queue.front().unwrap() {
                // Ping?
                0x55 => {
                    println!("KBD: Ping");
                    kbd_queue.pop_front();
                    _ = in_kbd.send(1);
                    _ = in_kbd.send(0);
                    _ = in_kbd.send(0);
                    _ = in_kbd.send(0);
                }
                0x11 | 0x13 => {
                    if kbd_queue.len() >= 2 {
                        println!("KBD: LED state");
                        kbd_queue.pop_front();
                        kbd_queue.pop_front();
                    }
                }
                0x99 => {
                    println!("KBD: disable click");
                    kbd_queue.pop_front();
                    _ = in_kbd.send(0xba);
                }
                0xAB => {
                    println!("KBD: Request ID");
                    kbd_queue.pop_front();
                    _ = in_kbd.send(1);
                    _ = in_kbd.send(0);
                }
                0xE1 => {
                    println!("KBD: Disable repeat");
                    kbd_queue.pop_front();
                    _ = in_kbd.send(0xba);
                }
                0xE3 => {
                    println!("KBD: Enable repeat");
                    kbd_queue.pop_front();
                    _ = in_kbd.send(0xba);
                }
                0xFD => {
                    println!("KBD: Power-up");
                    kbd_queue.pop_front();
                    _ = in_kbd.send(1);
                    _ = in_kbd.send(0);
                    _ = in_kbd.send(0);
                    _ = in_kbd.send(0);
                }
                _ => {
                    println!("KBD (unknown): {:02X}", value);
                    kbd_queue.clear();
                    _ = in_kbd.send(0xB6);
                }
            }
        }
        breakpoints.run(true, &mut cpu, &mut code);
        cpu.step(&mut ram, &mut code, &mut ports);
        breakpoints.run(false, &mut cpu, &mut code);

        if pc == 0x5d99 || pc == 0x5DBA {
            // cpu.interrupt = true;
            // cpu.push_stack16(cpu.pc);
            // cpu.pc = 0x23;
            // cpu.sfr_set(SFR_SCON, 1, &mut ());
        }

        instruction_count += 1;

        if args.debug && instruction_count % 100 == 0 {
            println!(
                "Executed {} instructions, PC = 0x{:04X}",
                instruction_count, cpu.pc
            );
        }

        // Check for infinite loops or other issues
        if cpu.pc as usize >= code.rom_size() {
            println!(
                "PC out of ROM range: 0x{:04X} (ROM size: 0x{:04X})",
                cpu.pc,
                code.rom_size()
            );
            break;
        }

        // if cpu.pc == pc && op == 0x80 {
        //     println!("Halting CPU at 0x{:04X}", cpu.pc);
        //     break;
        // }

        // Safety check for infinite loops
        if instruction_count > 0 && cpu.pc == 0 {
            println!(
                "PC reset to 0 from 0x{:04X}, possible infinite loop detected",
                pc
            );
            break;
        }
    }

    eprintln!("RAM: {:X?}", &ram.ram[0x8000..0x8004]);

    let elapsed = start_time.elapsed();
    println!("\nCPU execution completed:");
    println!("  Instructions executed: {}", instruction_count);
    println!("  Final PC: 0x{:04X}", cpu.pc);
    println!("  Time elapsed: {:?}", elapsed);
    if elapsed.as_secs_f64() > 0.0 {
        println!(
            "  Instructions per second: {:.0}",
            instruction_count as f64 / elapsed.as_secs_f64()
        );
    }

    println!("\nVT420 emulator execution completed!");

    Ok(())
}
