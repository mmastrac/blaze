use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use i8051::Cpu;
use i8051_debug_tui::{Debugger, DebuggerState};
use ratatui::crossterm;

use crate::{
    System,
    machine::vt420::video::{decode_font, decode_vram},
};

#[derive(Default)]
pub struct WgpuRender {}

impl WgpuRender {
    pub fn render(&self, system: &System, frame: &mut [u8]) {
        // Don't render during vsync
        if system.memory.mapper.get(6) & 0xf0 == 0xf0 {
            return;
        }

        #[derive(Default)]
        struct Render<'a> {
            row: usize,
            row_offset: usize,
            row_height: usize,
            is_80: bool,
            double_width: bool,
            status_row: bool,
            screen_2: bool,
            font: u8,
            frame: &'a mut [u8],
            invert: bool,
        }
        let render = Render {
            screen_2: system.memory.mapper.is_screen_2(),
            frame,
            ..Default::default()
        };
        let mut font = [0_u16; 16];
        decode_vram(
            &system.memory.vram,
            &system.memory.mapper,
            |render, row, attr, row_height| {
                render.row += render.row_height;
                render.row_offset += 800 * 4 * render.row_height;
                render.row_height = row_height as usize;
                render.double_width = attr == 0x4;
                if attr & 0x02 != 0 {
                    render.screen_2 = !render.screen_2;
                }
                // TODO: This flashes like crazy, something isn't quite right
                // render.invert = if render.screen_2 {
                //     system.memory.mapper.screen_2_invert()
                // } else {
                //     system.memory.mapper.screen_1_invert()
                // };
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
                let underline = attr & 1 != 0;
                let color = if bold { 0xff } else { 0x80 };
                let mut font_address_base = c * 16 + 0x8000 + render.font as usize * 0x80;
                if !render.is_80 {
                    font_address_base += 16;
                }
                decode_font(
                    &system.memory.vram,
                    font_address_base as _,
                    render.is_80,
                    &mut font,
                );
                let width = if render.is_80 { 10 } else { 6 };
                for y in 0..render.row_height {
                    if render.row + y >= 416 {
                        break;
                    }
                    let offset = render.row_offset + y * 800 * 4;
                    if render.double_width {
                        for x in 0..width {
                            let x_offset = (column as usize * width + x) * 8;
                            let mut pixel = font[y] & (1 << x) != 0;
                            if underline && y == render.row_height - 1 {
                                pixel = true;
                            }
                            if attr & 16 != 0 {
                                pixel = !pixel;
                            }
                            let color = if pixel ^ render.invert { color } else { 0x00 };
                            render.frame[offset + x_offset] = color;
                            render.frame[offset + x_offset + 1] = color;
                            render.frame[offset + x_offset + 2] = color;
                            render.frame[offset + x_offset + 3] = 0xff;
                            render.frame[offset + x_offset + 4] = color;
                            render.frame[offset + x_offset + 5] = color;
                            render.frame[offset + x_offset + 6] = color;
                            render.frame[offset + x_offset + 7] = 0xff;
                        }
                    } else {
                        for x in 0..width {
                            let x_offset = (column as usize * width + x) * 4;
                            let mut pixel = font[y] & (1 << x) != 0;
                            if underline && y == render.row_height - 1 {
                                pixel = true;
                            }
                            if attr & 16 != 0 {
                                pixel = !pixel;
                            }
                            let color = if pixel ^ render.invert { color } else { 0x00 };
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
    }
}

pub fn run(
    system: System,
    mut cpu: Cpu,
    debugger: Option<Debugger>,
) -> Result<usize, Box<dyn std::error::Error>> {
    if let Some(debugger) = debugger {
        return run_debugger(system, cpu, debugger);
    }

    let sender = system.keyboard.sender();
    let system = Rc::new(RefCell::new(system));
    let render = crate::host::screen::wgpu::WgpuRender::default();

    let system_clone = system.clone();
    let stepper = move || {
        let mut system = system_clone.borrow_mut();
        for _ in 0..20000 {
            system.step(&mut cpu);
        }
    };

    let system_clone = system.clone();
    crate::host::wgpu::main(
        sender,
        move |frame| render.render(&system_clone.borrow(), frame),
        stepper,
    )?;

    return Ok(system.borrow().instruction_count);
}

fn run_debugger(
    system: System,
    mut cpu: Cpu,
    mut debugger: Debugger,
) -> Result<usize, Box<dyn std::error::Error>> {
    debugger.enter()?;

    let sender = system.keyboard.sender();
    let system = Rc::new(RefCell::new(system));
    let render = crate::host::screen::wgpu::WgpuRender::default();

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
        }
    };

    let system_clone = system.clone();
    crate::host::wgpu::main(
        sender,
        move |frame| render.render(&system_clone.borrow(), frame),
        stepper,
    )?;

    return Ok(system.borrow().instruction_count);
}
