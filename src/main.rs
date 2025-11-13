use clap::Parser;
#[cfg(feature = "tui")]
use i8051_debug_tui::{Debugger, TracingCollector};
use std::path::PathBuf;
use tracing::{Level, info};

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

mod host;
mod machine;

use machine::vt420::System;
use machine::vt420::breakpoints::create_breakpoints;

use i8051::Cpu;

use crate::host::comm::CommConfig;

#[derive(Default, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum Display {
    /// No display. Runs the emulator in headless mode.
    #[default]
    Headless,
    /// Display the video output in a text-based UI.
    #[cfg(feature = "tui")]
    Text,
    /// Display the video output in a graphical UI.
    #[cfg(feature = "graphics")]
    Graphics,
}

/// VT420 Terminal Emulator
/// Emulates a VT420 terminal using an 8051 microcontroller
#[derive(Default, Parser)]
#[command(name = "vt-emulator")]
#[command(about = "A VT420 terminal emulator using 8051 CPU emulation")]
struct Args {
    /// Path to the ROM file
    #[arg(long)]
    #[cfg(not(feature = "embed-rom"))]
    rom: PathBuf,

    /// Path to the non-volatile RAM file
    #[arg(long)]
    nvr: Option<PathBuf>,

    /// Display the video output
    #[arg(long, conflicts_with = "benchmark")]
    display: Option<Display>,

    /// Comm1: Single bidirectional pipe
    #[arg(long = "comm1-pipe", value_name = "PIPE", group = "comm1")]
    comm1_pipe: Option<PathBuf>,

    /// Comm1: Separate read and write pipes
    #[arg(long = "comm1-pipes", num_args = 2, value_names = ["RX", "TX"], group = "comm1")]
    comm1_pipes: Vec<PathBuf>,

    /// Comm1: Execute a command and connect to its stdin/stdout
    #[arg(long = "comm1-exec-raw", value_name = "COMMAND", group = "comm1")]
    comm1_exec_raw: Option<String>,

    /// Comm1: Execute a command and connect to its pty
    #[arg(long = "comm1-exec", value_name = "COMMAND", group = "comm1")]
    comm1_exec: Option<String>,

    /// Comm1: Use loopback mode
    #[arg(long = "comm1-loopback", group = "comm1")]
    comm1_loopback: bool,

    /// Comm2: Single bidirectional pipe
    #[arg(long = "comm2-pipe", value_name = "PIPE", group = "comm2")]
    comm2_pipe: Option<PathBuf>,

    /// Comm2: Separate read and write pipes
    #[arg(long = "comm2-pipes", num_args = 2, value_names = ["RX", "TX"], group = "comm2")]
    comm2_pipes: Vec<PathBuf>,

    /// Comm2: Execute a command and connect to its stdin/stdout
    #[arg(long = "comm2-exec-raw", value_name = "COMMAND", group = "comm2")]
    comm2_exec_raw: Option<String>,

    /// Comm2: Execute a command and connect to its pty
    #[arg(long = "comm2-exec", value_name = "COMMAND", group = "comm2")]
    comm2_exec: Option<String>,

    /// Comm2: Use loopback mode
    #[arg(long = "comm2-loopback", group = "comm2")]
    comm2_loopback: bool,

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

    /// Run the benchmark mode to see how many cycles we can hit
    #[arg(long, conflicts_with = "display")]
    benchmark: bool,
}

fn parse_hex_address(s: &str) -> Result<u32, Box<dyn std::error::Error + Send + Sync>> {
    Ok(u32::from_str_radix(s, 16)?)
}

fn setup_logging(args: &Args, #[cfg(feature = "tui")] trace_collector: TracingCollector) {
    let level = if args.verbose {
        Level::TRACE
    } else {
        Level::INFO
    };

    #[cfg(feature = "tui")]
    if args.debug {
        host::logging::setup_logging_debugger(level, trace_collector.clone());
        return;
    }

    match args.display.unwrap_or(Display::Headless) {
        Display::Headless => {
            host::logging::setup_logging_stdio(level);
        }
        #[cfg(feature = "graphics")]
        Display::Graphics => {
            host::logging::setup_logging_stdio(level);
        }
        #[cfg(feature = "tui")]
        Display::Text => {
            if args.log {
                host::logging::setup_logging_file(level);
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
fn start() {
    console_error_panic_hook::set_once();
    let mut config = tracing_wasm::WASMLayerConfigBuilder::new();
    config.set_max_level(Level::INFO);
    tracing_wasm::set_as_global_default_with_config(config.build());

    if let Err(e) = run(
        Args {
            display: Display::Graphics,
            ..Default::default()
        },
        #[cfg(feature = "tui")]
        TracingCollector::new(1000),
    ) {
        error!("Error: {}", e);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut args = Args::parse();

    // Set display to Headless if benchmark is set
    if args.benchmark {
        args.display = Some(Display::Headless);
    }

    #[cfg(feature = "tui")]
    let trace_collector = TracingCollector::new(1000);
    setup_logging(
        &args,
        #[cfg(feature = "tui")]
        trace_collector.clone(),
    );

    run(
        args,
        #[cfg(feature = "tui")]
        trace_collector,
    )
}

fn run(
    args: Args,
    #[cfg(feature = "tui")] trace_collector: TracingCollector,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    info!("VT420 Emulator starting...");

    #[cfg(feature = "embed-rom")]
    let rom = { include_bytes!("../roms/vt420/23-068E9-00.bin").to_vec() };
    #[cfg(not(feature = "embed-rom"))]
    let rom = {
        use std::fs;
        info!("Loading ROM file: {:?}...", args.rom);

        // Check if ROM file exists
        if !args.rom.exists() {
            info!("Error: ROM file does not exist: {:?}", args.rom);
            std::process::exit(1);
        }

        fs::read(&args.rom)?
    };

    info!("Configuring system...");

    // Parse comm1 configuration
    let comm1_pipes = if args.comm1_pipes.len() == 2 {
        Some((args.comm1_pipes[0].clone(), args.comm1_pipes[1].clone()))
    } else {
        None
    };
    let comm1_config = CommConfig::from_args(
        args.comm1_pipe,
        comm1_pipes,
        args.comm1_exec_raw,
        args.comm1_exec,
        args.comm1_loopback,
    );

    // Parse comm2 configuration
    let comm2_pipes = if args.comm2_pipes.len() == 2 {
        Some((args.comm2_pipes[0].clone(), args.comm2_pipes[1].clone()))
    } else {
        None
    };
    let comm2_config = CommConfig::from_args(
        args.comm2_pipe,
        comm2_pipes,
        args.comm2_exec_raw,
        args.comm2_exec,
        args.comm2_loopback,
    );

    let mut system = System::new(rom, args.nvr.as_deref(), comm1_config, comm2_config)?;

    let breakpoints = &mut system.breakpoints;
    if args.log {
        create_breakpoints(breakpoints, &system.rom);
    }

    info!("Starting CPU execution...");
    let mut cpu = Cpu::new();
    #[cfg(not(target_arch = "wasm32"))]
    let start_time = Instant::now();
    info!("CPU initialized, PC = 0x{:04X}", cpu.pc_ext(&system));

    #[cfg(feature = "tui")]
    let debugger = if args.debug {
        let mut debugger = Debugger::new(Default::default(), trace_collector)?;
        for breakpoint in args.breakpoint {
            debugger.breakpoints_mut().insert(breakpoint);
        }
        Some(debugger)
    } else {
        None
    };

    let instruction_count = if args.benchmark {
        for _ in 0..100_000_000 {
            system.step(&mut cpu);
        }
        system.instruction_count
    } else {
        match args.display.unwrap_or(Display::Headless) {
            Display::Headless => host::screen::headless::run(
                system,
                cpu,
                #[cfg(feature = "tui")]
                debugger,
            )?,
            #[cfg(feature = "tui")]
            Display::Text => {
                host::screen::ratatui::run(system, cpu, debugger, args.show_mapper, args.show_vram)?
            }
            #[cfg(feature = "graphics")]
            Display::Graphics => host::screen::wgpu::run(
                system,
                cpu,
                #[cfg(feature = "tui")]
                debugger,
            )?,
        }
    };

    #[cfg(not(target_arch = "wasm32"))]
    let elapsed = start_time.elapsed();
    println!("CPU execution completed:");
    println!("  Instructions executed: {}", instruction_count);
    #[cfg(not(target_arch = "wasm32"))]
    println!("  Time elapsed: {:?}", elapsed);
    #[cfg(not(target_arch = "wasm32"))]
    if elapsed.as_secs_f64() > 0.0 {
        let ips = instruction_count as f64 / elapsed.as_secs_f64();
        println!("  Instructions per second: {ips:.0}",);
        println!("  % of real CPU: {:.0}%", ips / 1000000.0 * 100.0);
    }

    println!("VT420 emulator execution completed!");

    Ok(())
}
