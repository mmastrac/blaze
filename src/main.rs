use clap::Parser;
use i8051_debug_tui::{Debugger, TracingCollector};
use std::path::PathBuf;
use std::time::Instant;
use tracing::{Level, info};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

mod host;
mod machine;

use machine::vt420::{System, breakpoints::create_breakpoints};

use i8051::Cpu;

use crate::host::comm::CommConfig;

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum Display {
    /// No display. Runs the emulator in headless mode.
    Headless,
    /// Display the video output in a text-based UI.
    Text,
    /// Display the video output in a graphical UI.
    #[cfg(feature = "graphics")]
    Graphics,
}

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
    display: Display,

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

    /// Enable logging
    #[arg(long)]
    log: bool,

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

    let trace_collector = TracingCollector::new(1000);
    if args.debug {
        host::logging::setup_logging_debugger(level, trace_collector.clone());
    } else {
        match args.display {
            Display::Headless => {
                host::logging::setup_logging_stdio(level);
            }
            #[cfg(feature = "graphics")]
            Display::Graphics => {
                host::logging::setup_logging_stdio(level);
            }
            Display::Text => {
                if args.log {
                    host::logging::setup_logging_file(level);
                }
            }
        }
    }

    info!("VT420 Emulator starting...");
    info!("ROM file: {:?}", args.rom);

    // Check if ROM file exists
    if !args.rom.exists() {
        info!("Error: ROM file does not exist: {:?}", args.rom);
        std::process::exit(1);
    }

    info!("Configuring system...");

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

    let breakpoints = &mut system.breakpoints;
    if args.log {
        create_breakpoints(breakpoints, &system.rom);
    }

    info!("Starting CPU execution...");
    let cpu = Cpu::new();
    let start_time = Instant::now();
    info!("CPU initialized, PC = 0x{:04X}", cpu.pc_ext(&system));

    let debugger = if args.debug {
        let mut debugger = Debugger::new(Default::default(), trace_collector)?;
        for breakpoint in args.breakpoint {
            debugger.breakpoints_mut().insert(breakpoint);
        }
        Some(debugger)
    } else {
        None
    };

    let instruction_count = match args.display {
        Display::Headless => host::screen::headless::run(system, cpu, debugger)?,
        Display::Text => {
            host::screen::ratatui::run(system, cpu, debugger, args.show_mapper, args.show_vram)?
        }
        #[cfg(feature = "graphics")]
        Display::Graphics => host::screen::wgpu::run(system, cpu, debugger)?,
    };

    let elapsed = start_time.elapsed();
    info!("CPU execution completed:");
    info!("  Instructions executed: {}", instruction_count);
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
