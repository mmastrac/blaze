use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use i8051::Cpu;
#[cfg(feature = "tui")]
use i8051_debug_tui::{Debugger, DebuggerState};
#[cfg(feature = "tui")]
use ratatui::crossterm;

use crate::{
    System,
    machine::vt420::video::{RowFlags, decode_font, decode_vram},
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
            row_flags: RowFlags,
            start_row: usize,
            frame: &'a mut [u8],
            smooth: (u8, u8, u8),
        }
        let render = Render {
            smooth: (
                system.memory.mapper.get(0),
                system.memory.mapper.get(1),
                system.memory.mapper.get(2),
            ),
            frame,
            ..Default::default()
        };
        let mut font = [0_u16; 16];
        let render = decode_vram(
            &system.memory.vram[system.memory.mapper.vram_offset_display() as usize..],
            &system.memory.mapper,
            |render, row, attr, row_flags| {
                render.row += render.row_flags.row_height as usize;
                render.row_offset += 800 * 4 * render.row_flags.row_height as usize;

                render.row_flags = row_flags;
                render.start_row = 0;
                if render.smooth.2 != 0 {
                    if (render.smooth.0..=render.smooth.1).contains(&row) {
                        if row == render.smooth.0 {
                            render.start_row = render.smooth.2 as usize;
                            render.row_flags.row_height =
                                render.row_flags.row_height - render.smooth.2;
                        } else if row == render.smooth.1 {
                            //render.start_row += 1;
                            render.row_flags.row_height = render.smooth.2;
                        }
                    }
                }
            },
            |render, column, c, attr| {
                let c = c as usize | ((((attr >> 2) & 0x01) as usize) << 8);
                let mut c = c * 2;
                if attr >> 2 & 0x8 != 0 && render.row_flags.status_row {
                    c = c.saturating_sub(1);
                }
                let bold = attr & 0x08 != 0;
                let underline = attr & 1 != 0;
                let color = if bold { 0xff } else { 0x80 };
                let mut font_address_base = c * 16 + 0x8000 + render.row_flags.font as usize * 0x80;
                if !render.row_flags.is_80 {
                    font_address_base += 16;
                }
                decode_font(
                    system.memory.vram.as_ref(),
                    font_address_base as _,
                    render.row_flags.is_80,
                    &mut font,
                );
                let width = if render.row_flags.is_80 { 10 } else { 6 };
                let mut offset = render.row_offset;
                for mut y in 0..render.row_flags.row_height as usize {
                    if render.row + y >= 416 {
                        break;
                    }
                    if c == 0 && !render.row_flags.is_80 {
                        // Stopgap to fix the leftover pixels at the end of the frame
                        const LEFTOVER_132_PIXELS: usize = 80 * 10 - 132 * 6;
                        for i in 0..LEFTOVER_132_PIXELS * 4 {
                            render.frame[offset + 800 * 4 - LEFTOVER_132_PIXELS * 4 + i] = 0;
                        }
                    }
                    if render.row_flags.double_width {
                        if render.row_flags.double_height_top {
                            y /= 2;
                        } else if render.row_flags.double_height_bottom {
                            y /= 2;
                            y += render.row_flags.row_height as usize / 2;
                        }
                        for x in 0..width {
                            let x_offset = (column as usize * width + x) * 8;
                            let mut pixel = font[y + render.start_row] & (1 << x) != 0;
                            if underline && y == render.row_flags.row_height as usize - 1 {
                                pixel = true;
                            }
                            if attr & 16 != 0 {
                                pixel = !pixel;
                            }
                            let color = if pixel ^ render.row_flags.invert {
                                color
                            } else {
                                0x00
                            };
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
                            let mut pixel = font[y + render.start_row] & (1 << x) != 0;
                            if underline && y == render.row_flags.row_height as usize - 1 {
                                pixel = true;
                            }
                            if attr & 16 != 0 {
                                pixel = !pixel;
                            }
                            let color = if pixel ^ render.row_flags.invert {
                                color
                            } else {
                                0x00
                            };
                            render.frame[offset + x_offset] = color;
                            render.frame[offset + x_offset + 1] = color;
                            render.frame[offset + x_offset + 2] = color;
                            render.frame[offset + x_offset + 3] = 0xff;
                        }
                    }
                    offset += 800 * 4;
                }
            },
            render,
        );

        // Stopgap to fix the leftover pixels at the end of the frame
        if render.row_offset < render.frame.len() {
            render.frame[render.row_offset..].fill(0);
        }
    }
}

pub fn run(
    system: System,
    mut cpu: Cpu,
    #[cfg(feature = "tui")] debugger: Option<Debugger>,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    #[cfg(feature = "tui")]
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
    )
    .map_err(|e| format!("Graphics error: {}", e))?;

    return Ok(system.borrow().instruction_count);
}

#[cfg(feature = "tui")]
fn run_debugger(
    system: System,
    mut cpu: Cpu,
    mut debugger: Debugger,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
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
