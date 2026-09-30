use clap::Parser;
#[cfg(feature = "tui")]
use i8051_debug_tui::{Debugger, TracingCollector};
use ssu::session::{SessionConfig, SessionPartsUnsend};
use std::path::PathBuf;
use tracing::{Level, info};

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

mod host;
pub mod machine;

#[cfg(all(feature = "pc-trace", not(target_arch = "wasm32")))]
mod pc_trace;

use i8051::Cpu;

use crate::machine::System;
use crate::machine::generic::script::Script;

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

#[derive(Default, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum MachineType {
    /// VT420
    #[default]
    VT420,
    /// VT520 or VT525
    VT52x,
    /// VT510
    VT510,
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

    /// Path to the ROM file
    #[arg(long)]
    #[cfg(feature = "embed-rom")]
    rom: Option<PathBuf>,

    /// Path to the non-volatile RAM file
    #[arg(long)]
    nvr: Option<PathBuf>,

    /// Display the video output
    #[arg(long, conflicts_with = "benchmark")]
    display: Option<Display>,

    /// Comm1 session configuration
    #[arg(long = "comm1", value_name = "SESSION")]
    comm1: Option<SessionConfig>,

    /// Comm2 session configuration
    #[arg(long = "comm2", value_name = "SESSION")]
    comm2: Option<SessionConfig>,

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

    /// Script of actions to run at given PCs
    #[arg(long, value_name = "FILE")]
    script: Option<PathBuf>,

    /// Enable logging
    #[arg(long)]
    log: bool,

    /// Enable verbose output
    #[arg(short, long)]
    verbose: bool,

    /// Run the benchmark mode to see how many cycles we can hit
    #[arg(long, conflicts_with = "display")]
    benchmark: bool,

    /// Skip diagnostics
    #[arg(long)]
    skip_diagnostics: bool,

    /// Machine type
    #[arg(long, default_value = "vt420")]
    machine: MachineType,

    /// Write a ROM-sized code hit map (byte per ROM offset: 00 = not hit, 01 = hit); merges with existing file
    #[cfg(all(feature = "pc-trace", not(target_arch = "wasm32")))]
    #[arg(long, value_name = "FILE")]
    pc_trace: Option<PathBuf>,
}

fn comm_session(
    script: Option<&Script>,
    channel: usize,
    config: Option<SessionConfig>,
) -> Result<Option<SessionPartsUnsend>, std::io::Error> {
    let inner = config.map(SessionConfig::start_unsend).transpose()?;
    Ok(match script {
        Some(script) => Some(script.comm_session(channel, inner)),
        None => inner,
    })
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
fn get_hash_param() -> Option<String> {
    use web_sys::window;
    window()
        .map(|window| window.location())
        .and_then(|location| location.hash().ok())
        .and_then(|hash| {
            if let Some(hash) = hash.strip_prefix('#') {
                Some(hash.replace("%20", " "))
            } else {
                None
            }
        })
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
fn start() {
    use std::str::FromStr;
    use tracing::error;
    use web_sys::window;

    console_error_panic_hook::set_once();
    let mut config = tracing_wasm::WASMLayerConfigBuilder::new();
    config.set_max_level(Level::INFO);
    tracing_wasm::set_as_global_default_with_config(config.build());

    // If comm1 is set in the window's hash, use it
    let comm1 = if let Some(comm1) = get_hash_param() {
        Some(SessionConfig::from_str(&comm1).unwrap())
    } else {
        None
    };

    if let Err(e) = run_vt420(
        Args {
            display: Some(Display::Graphics),
            comm1,
            skip_diagnostics: true,
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

    match args.machine {
        MachineType::VT420 => run_vt420(
            args,
            #[cfg(feature = "tui")]
            trace_collector,
        ),
        MachineType::VT52x => run_vt52x(
            args,
            #[cfg(feature = "embed-rom")]
            include_bytes!("../roms/vt520/23-010ED-00.bin"),
            #[cfg(feature = "tui")]
            trace_collector,
        ),
        MachineType::VT510 => run_vt52x(
            args,
            #[cfg(feature = "embed-rom")]
            include_bytes!("../roms/vt510/23-032ED-00.bin"),
            #[cfg(feature = "tui")]
            trace_collector,
        ),
    }
}

fn run_vt420(
    args: Args,
    #[cfg(feature = "tui")] trace_collector: TracingCollector,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use machine::vt420::breakpoints::create_breakpoints;

    info!("VT420 Emulator starting...");

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

    #[cfg(feature = "embed-rom")]
    let mut rom = { include_bytes!("../roms/vt420/23-068E9-00.bin").to_vec() };
    #[cfg(feature = "embed-rom")]
    if let Some(rom_path) = args.rom {
        use std::fs;
        info!("Loading ROM file: {:?}...", rom_path);

        // Check if ROM file exists
        if !rom_path.exists() {
            info!("Error: ROM file does not exist: {:?}", rom_path);
            std::process::exit(1);
        }

        rom = fs::read(&rom_path)?;
    };

    info!("Configuring system...");

    let script = args.script.as_deref().map(Script::load).transpose()?;
    let mut vt420 = machine::vt420::System::new(
        rom,
        args.nvr.as_deref(),
        comm_session(script.as_ref(), 0, args.comm1)?,
        comm_session(script.as_ref(), 1, args.comm2)?,
        #[cfg(all(feature = "pc-trace", not(target_arch = "wasm32")))]
        args.pc_trace.as_deref(),
    )?;
    if let Some(script) = script {
        vt420.script = script;
    }
    let mut system = System::new(vt420);

    let breakpoints = &mut system.system.breakpoints;
    if args.log {
        create_breakpoints(breakpoints);
    }

    info!("Starting CPU execution...");
    let mut cpu = Cpu::new();
    #[cfg(not(target_arch = "wasm32"))]
    let start_time = Instant::now();
    info!("CPU initialized, PC = 0x{:04X}", cpu.pc_ext(&system));

    if args.skip_diagnostics {
        // TODO: This should be more heuristic
        for _ in 0..0x800_000_u64 {
            system.step(&mut cpu);
        }
        #[cfg(all(feature = "pc-trace", not(target_arch = "wasm32")))]
        system.flush_pc_trace_if_due();
    }

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
        for _i in 0..100_000_000 {
            system.step(&mut cpu);
            #[cfg(all(feature = "pc-trace", not(target_arch = "wasm32")))]
            if _i & 0xFFFF == 0 {
                system.flush_pc_trace_if_due();
            }
        }
        #[cfg(all(feature = "pc-trace", not(target_arch = "wasm32")))]
        system.flush_pc_trace_if_due();
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
            Display::Text => host::screen::ratatui::run(
                system.system,
                cpu,
                debugger,
                args.show_mapper,
                args.show_vram,
            )?,
            #[cfg(feature = "graphics")]
            Display::Graphics => host::screen::framebuffer::run(
                system.system,
                cpu,
                #[cfg(feature = "tui")]
                debugger,
            )?,
        }
    };

    #[cfg(not(target_arch = "wasm32"))]
    let elapsed = start_time.elapsed();
    println!("CPU execution completed:");
    println!("  Instructions executed: {instruction_count}");
    #[cfg(not(target_arch = "wasm32"))]
    println!("  Time elapsed: {elapsed:?}");
    #[cfg(not(target_arch = "wasm32"))]
    if elapsed.as_secs_f64() > 0.0 {
        let ips = instruction_count as f64 / elapsed.as_secs_f64();
        println!("  Instructions per second: {ips:.0}",);
        println!("  % of real CPU: {:.0}%", ips / 1000000.0 * 100.0);
    }

    println!("VT420 emulator execution completed!");

    Ok(())
}

fn run_vt52x(
    args: Args,
    #[cfg(feature = "embed-rom")] embedded_rom: &'static [u8],
    #[cfg(feature = "tui")] trace_collector: TracingCollector,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    info!("VT52x Emulator starting...");

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

    #[cfg(feature = "embed-rom")]
    let mut rom = embedded_rom.to_vec();
    #[cfg(feature = "embed-rom")]
    if let Some(rom_path) = args.rom {
        use std::fs;
        info!("Loading ROM file: {:?}...", rom_path);

        // Check if ROM file exists
        if !rom_path.exists() {
            info!("Error: ROM file does not exist: {:?}", rom_path);
            std::process::exit(1);
        }

        rom = fs::read(&rom_path)?;
    };

    info!("Configuring system...");

    let script = args.script.as_deref().map(Script::load).transpose()?;
    let default = || script.is_none().then(SessionConfig::default);
    let mut vt52x = machine::vt52x::System::new(
        rom,
        args.nvr.as_deref(),
        comm_session(script.as_ref(), 0, args.comm1.or_else(default))?,
        comm_session(script.as_ref(), 1, args.comm2.or_else(default))?,
    )?;
    if let Some(script) = script {
        vt52x.script = script;
    }
    let mut system = System::new(vt52x);

    info!("Starting CPU execution...");
    let mut cpu = Cpu::new();
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

    if args.benchmark {
        for _ in 0..100_000_000 {
            system.step(&mut cpu);
        }
    } else {
        match args.display.unwrap_or(Display::Headless) {
            Display::Headless => {
                host::screen::headless::run(
                    system,
                    cpu,
                    #[cfg(feature = "tui")]
                    debugger,
                )?;
            }
            #[allow(unreachable_patterns)]
            _ => {
                unimplemented!()
            }
        }
    }

    Ok(())
}
