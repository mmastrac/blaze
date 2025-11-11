use clap::Parser;
use i8051::sfr::{SFR_P1, SFR_P2, SFR_P3};
use ratatui::layout::Offset;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use std::fs::{self, File};
use std::io::{self, stdout};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tracing::{Level, info, warn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm;

mod host;
mod machine;

use machine::vt420::{System, breakpoints::create_breakpoints};

use i8051::Cpu;

use crate::host::comm::CommConfig;
use crate::host::lk201::crossterm::KeyboardCommand;
use crate::host::screen::{DisplayMode, Screen};
use crate::machine::vt420;

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

    if !args.debug {
        if args.display {
            if cfg!(feature = "graphics") {
                host::logging::setup_logging_stdio(level);
            } else if args.log {
                host::logging::setup_logging_file(level);
            }
        } else {
            host::logging::setup_logging_stdio(level);
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
    let mut instruction_count = 0_usize;

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
            #[cfg(feature = "graphics")]
            {
                use std::cell::RefCell;
                use std::rc::Rc;

                let sender = system.keyboard.sender();
                let system = Rc::new(RefCell::new(system));
                let system_clone = system.clone();
                let mut render = move |frame: &mut [u8]| {
                    let system = &*system_clone.borrow();
                    if system.memory.mapper.get(6) & 0xf0 == 0xf0 {
                        return;
                    }
                    struct Render<'a> {
                        row: usize,
                        row_offset: usize,
                        row_height: usize,
                        is_80: bool,
                        status_row: bool,
                        screen_2: bool,
                        font: u8,
                        frame: &'a mut [u8],
                        invert: bool,
                    }
                    let mut render = Render {
                        row: 0,
                        row_offset: 0,
                        row_height: 0,
                        is_80: false,
                        status_row: false,
                        screen_2: system.memory.mapper.is_screen_2(),
                        font: 0,
                        frame,
                        invert: false,
                    };
                    vt420::video::decode_vram(
                        &system.memory.vram,
                        &system.memory.mapper,
                        |render, row, attr, row_height| {
                            render.row += render.row_height;
                            render.row_offset += 800 * 4 * render.row_height;
                            render.row_height = row_height as usize;
                            if attr & 0x02 != 0 {
                                render.screen_2 = !render.screen_2;
                            }
                            render.invert = if render.screen_2 {
                                system.memory.mapper.screen_2_invert()
                            } else {
                                system.memory.mapper.screen_1_invert()
                            };
                            render.font = if render.screen_2 {
                                system.memory.mapper.get(0xc) & 0xf0
                            } else {
                                system.memory.mapper.get2(0xc) & 0xf0
                            };
                            render.is_80 = !if render.screen_2 {
                                system.memory.mapper.screen_2_132_columns()
                            } else {
                                system.memory.mapper.screen_1_132_columns()
                            };
                            // Passing through status row in this attribute
                            render.status_row = attr & 0x80 != 0;
                            if render.status_row {
                                render.is_80 = false;
                            }
                        },
                        |render, column, c, attr| {
                            let c = c as usize | ((((attr >> 2) & 0x01) as usize) << 8);
                            let mut c = c * 2;
                            if attr >> 2 & 0x8 != 0 && render.status_row {
                                c = c.saturating_sub(1);
                            }
                            let bold = attr & 0x08 != 0;
                            let color = if bold { 0xff } else { 0x80 };
                            let mut font_address_base =
                                c * 16 + 0x8000 + render.font as usize * 0x80;
                            if render.is_80 {
                                let font_address = font_address_base;
                                for y in 0..render.row_height {
                                    if render.row + y >= 416 {
                                        break;
                                    }
                                    let offset = render.row_offset + y * 800 * 4;
                                    for x in 0..10 {
                                        let x_offset = (column as usize * 10 + x) * 4;
                                        let mut pixel = if x >= 8 {
                                            system.memory.vram[font_address + y + 16]
                                                & (1 << (x - 8))
                                                != 0
                                        } else {
                                            system.memory.vram[font_address + y] & (1 << x) != 0
                                        };
                                        if attr & 16 != 0 {
                                            pixel = !pixel;
                                        }
                                        let color =
                                            if pixel ^ render.invert { color } else { 0x00 };
                                        render.frame[offset + x_offset] = color;
                                        render.frame[offset + x_offset + 1] = color;
                                        render.frame[offset + x_offset + 2] = color;
                                        render.frame[offset + x_offset + 3] = 0xff;
                                    }
                                }
                            } else {
                                let font_address = font_address_base + 16;
                                for y in 0..render.row_height {
                                    if render.row + y >= 416 {
                                        break;
                                    }
                                    let offset = render.row_offset + y * 800 * 4;
                                    for x in 0..6 {
                                        let x_offset = (column as usize * 6 + x) * 4;
                                        let mut pixel = system.memory.vram[font_address + y]
                                            & (1 << (x + 2))
                                            != 0;
                                        if attr & 16 != 0 {
                                            pixel = !pixel;
                                        }
                                        let color =
                                            if pixel ^ render.invert { color } else { 0x00 };
                                        render.frame[offset + x_offset] = color;
                                        render.frame[offset + x_offset + 1] = color;
                                        render.frame[offset + x_offset + 2] = color;
                                        render.frame[offset + x_offset + 3] = 0xff;
                                    }
                                }
                            }
                        },
                        render,
                    );
                };

                let mut step = move || {
                    for _ in 0..20000 {
                        system.borrow_mut().step(&mut cpu);
                    }
                };

                host::wgpu::main(sender, render, step);
                return Ok(());
            }

            crossterm::terminal::enable_raw_mode()?;
            crossterm::execute!(io::stdout(), crossterm::terminal::EnterAlternateScreen,)?;
            terminal.clear()?;
        }

        // CPU execution loop
        let mut running = true;
        let mut pc_trace = false;
        let mut hex = DisplayMode::Normal;
        let mut keyboard = host::lk201::crossterm::CrosstermKeyboard::default();
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
                    match keyboard.update_keyboard(&event, &system.keyboard.sender()) {
                        Some(KeyboardCommand::ToggleRun) => {
                            running = !running;
                        }
                        Some(KeyboardCommand::ToggleHexMode) => {
                            hex = match hex {
                                DisplayMode::Normal => DisplayMode::NibbleTriplet,
                                DisplayMode::NibbleTriplet => DisplayMode::Bytes,
                                DisplayMode::Bytes => DisplayMode::Normal,
                            };
                        }
                        Some(KeyboardCommand::DumpVRAM) => {
                            fs::write("/tmp/vram.bin", &system.memory.vram[0..])?;
                        }
                        #[cfg(feature = "pc-trace")]
                        Some(KeyboardCommand::TogglePCTrace) => {
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
                        Some(KeyboardCommand::Quit) => {
                            break;
                        }
                        None => {}
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
                            let vram = &system.memory.vram;
                            for i in 0..16 {
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
                                        y: (f.area().height as i32 - 16) + i as i32,
                                    }),
                                );
                            }
                        }
                    })?;
                }
            }
        }
    }

    if args.display {
        crossterm::terminal::disable_raw_mode()?;
        crossterm::execute!(io::stdout(), crossterm::terminal::LeaveAlternateScreen)?;
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
