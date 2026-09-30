use std::cell::RefCell;
#[cfg(all(feature = "vram-dump", feature = "tui"))]
use std::io::Write;
use std::rc::Rc;
#[cfg(feature = "tui")]
use std::time::Duration;

use i8051::Cpu;
#[cfg(feature = "tui")]
use i8051_debug_tui::{Debugger, DebuggerState};
#[cfg(feature = "tui")]
use ratatui::crossterm;
#[cfg(feature = "tui")]
use ratatui::crossterm::event::KeyModifiers;
#[cfg(all(feature = "vram-dump", feature = "tui"))]
use tracing::info;

use crate::machine::TerminalSystem;
use crate::machine::generic::display::Display;
use crate::machine::generic::keyboard::lk201_input::Lk201Input;
use crate::machine::vt420::System;

pub fn run(
    system: System,
    mut cpu: Cpu,
    #[cfg(feature = "tui")] debugger: Option<Debugger>,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    #[cfg(feature = "tui")]
    if let Some(debugger) = debugger {
        return run_debugger(system, cpu, debugger);
    }

    let keyboard = Box::new(Lk201Input::new(system.keyboard.sender()));
    let system = Rc::new(RefCell::new(system));

    let system_clone = system.clone();
    #[cfg(all(feature = "vram-dump", feature = "tui"))]
    let mut terminal_keys = true;
    let stepper = move || {
        let mut system = system_clone.borrow_mut();
        if system.memory.mapper.disable_chargen() {
            // Run way faster if the chargen is disabled
            for _ in 0..100000 {
                system.step(&mut cpu);
            }
        } else {
            for _ in 0..20000 {
                system.step(&mut cpu);
            }
        }
        while system.memory.mapper.is_status_bar_phase() {
            system.step(&mut cpu);
        }
        if let Some(code) = system.exit_code() {
            std::process::exit(code);
        }
        #[cfg(all(feature = "vram-dump", feature = "tui"))]
        {
            let ready = terminal_keys
                && match crossterm::event::poll(Duration::from_millis(0)) {
                    Ok(ready) => ready,
                    Err(e) => {
                        info!("No terminal input, VRAM dump and screenshot keys disabled: {e}");
                        terminal_keys = false;
                        false
                    }
                };
            if ready {
                let Ok(event) = crossterm::event::read() else {
                    return;
                };
                match event {
                    crossterm::event::Event::Key(crossterm::event::KeyEvent {
                        code: crossterm::event::KeyCode::Char('d'),
                        modifiers: KeyModifiers::NONE,
                        ..
                    }) => {
                        let mut file = std::fs::File::create("/tmp/vram.bin").unwrap();
                        file.write_all(&system.memory.vram[..]).unwrap();
                        let mut file = std::fs::File::create("/tmp/vram.png").unwrap();
                        let w = system.memory.vram.len() as u32 / 256 * 8;
                        info!("Logging VRAM as {w}x256 to /tmp/vram.png");
                        let mut encoder = png::Encoder::new(&mut file, w, 256);
                        encoder.set_color(png::ColorType::Grayscale);
                        encoder.set_depth(png::BitDepth::Eight);
                        let mut writer = encoder.write_header().unwrap();
                        let mut row_data = Vec::with_capacity(w as usize * 256);
                        for row in 0..256 {
                            for col in 0..w {
                                let index = row + col / 8 * 256;
                                let pixel = system.memory.vram[index as usize];
                                row_data.push(if pixel & (1 << (col % 8)) != 0 {
                                    255
                                } else {
                                    0
                                });
                            }
                        }
                        writer.write_image_data(&row_data).unwrap();
                        info!("VRAM logged to /tmp/vram.png");
                    }
                    crossterm::event::Event::Key(crossterm::event::KeyEvent {
                        code: crossterm::event::KeyCode::Char('s'),
                        modifiers: KeyModifiers::NONE,
                        ..
                    }) => {
                        let mut file = std::fs::File::create("/tmp/screenshot.png").unwrap();
                        let w = crate::host::wgpu::REAL_WIDTH as u32;
                        let h = crate::host::wgpu::REAL_HEIGHT as u32;
                        let mut encoder = png::Encoder::new(&mut file, w, h);
                        encoder.set_color(png::ColorType::Rgba);
                        encoder.set_depth(png::BitDepth::Eight);
                        let mut writer = encoder.write_header().unwrap();
                        let mut row_data = vec![0; w as usize * h as usize * 4];
                        system.render_framebuffer(&mut row_data);
                        writer.write_image_data(&row_data).unwrap();
                        info!("Screenshot logged to /tmp/screenshot.png");
                    }
                    _ => {}
                }
            }
        }
        #[cfg(all(feature = "pc-trace", not(target_arch = "wasm32")))]
        if let Some(trace) = &mut system.pc_trace {
            trace.flush_if_due();
        }
    };

    let system_clone = system.clone();
    crate::host::wgpu::main(
        "VT420",
        keyboard,
        move |frame| system_clone.borrow().render_framebuffer(frame),
        stepper,
    )
    .map_err(|e| format!("Graphics error: {e}"))?;

    return Ok(system.borrow().instruction_count);
}

#[cfg(feature = "tui")]
fn run_debugger(
    system: System,
    mut cpu: Cpu,
    mut debugger: Debugger,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    debugger.enter()?;

    let keyboard = Box::new(Lk201Input::new(system.keyboard.sender()));
    let system = Rc::new(RefCell::new(system));

    let system_clone = system.clone();
    let stepper = move || {
        let system = &mut *system_clone.borrow_mut();
        debugger.render(&cpu, system).unwrap();
        if crossterm::event::poll(Duration::from_millis(0)).unwrap() {
            let Ok(event) = crossterm::event::read() else {
                return;
            };
            if debugger.handle_event(event, &mut cpu, system) {
                system.step(&mut cpu);
            }
            debugger.render(&cpu, system).unwrap();
        }
        for _ in 0..20000 {
            match debugger.debugger_state() {
                DebuggerState::Running => {
                    system.step(&mut cpu);
                }
                DebuggerState::Paused => {
                    return;
                }
                DebuggerState::Quit => {
                    return;
                }
            }
            if debugger.breakpoints().contains(&cpu.pc_ext(system)) {
                debugger.pause();
            }
            if let Some(code) = system.exit_code() {
                debugger.exit().unwrap();
                std::process::exit(code);
            }
        }
        #[cfg(all(feature = "pc-trace", not(target_arch = "wasm32")))]
        if let Some(trace) = &mut system.pc_trace {
            trace.flush_if_due();
        }
    };

    let system_clone = system.clone();
    crate::host::wgpu::main(
        "VT420",
        keyboard,
        move |frame| system_clone.borrow().render_framebuffer(frame),
        stepper,
    )?;

    return Ok(system.borrow().instruction_count);
}
