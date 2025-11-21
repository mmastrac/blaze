use std::fs::{self, File};
use std::io;
use std::time::{Duration, Instant};

use i8051::Cpu;
use i8051_debug_tui::Debugger;
use ratatui::buffer::Buffer;
use ratatui::crossterm;
use ratatui::layout::Offset;
use ratatui::layout::Rect;
use ratatui::prelude::CrosstermBackend;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use i8051::sfr::{SFR_P1, SFR_P2, SFR_P3};
use tracing::warn;

use crate::host::lk201::crossterm::{CrosstermKeyboard, KeyboardCommand};
use crate::host::screen::unicode;
use crate::machine::vt420::video::{RowFlags, decode_vram};
use crate::{System, machine::vt420::video::Mapper};

pub struct Screen<'a> {
    vram: &'a [u8],
    mapper: &'a Mapper,
    display_mode: DisplayMode,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DisplayMode {
    Normal,
    NibbleTriplet,
    Bytes,
}

impl<'a> Screen<'a> {
    pub fn new(vram: &'a [u8], mapper: &'a Mapper) -> Self {
        Self {
            vram,
            mapper,
            display_mode: DisplayMode::Normal,
        }
    }

    pub fn display_mode(mut self, mode: DisplayMode) -> Self {
        self.display_mode = mode;
        self
    }
}

impl<'a> Widget for Screen<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let vram = self.vram;
        let vram_base = 0;

        let mut line = [0_u16; 256];
        let mut attr = [0_u8; 256];

        #[derive(Default)]
        struct Render {
            row_idx: usize,
            row_flags: RowFlags,
            smooth_row: u8,
        }

        let render = Render {
            smooth_row: if self.mapper.get(2) != 0 {
                self.mapper.get(0)
            } else {
                u8::MAX
            },
            ..Default::default()
        };

        if self.display_mode == DisplayMode::Normal {
            decode_vram(
                self.vram,
                self.mapper,
                |render, row, attr, row_flags| {
                    render.row_idx = row as usize;
                    render.row_flags = row_flags;

                    if row >= render.smooth_row {
                        render.row_idx -= 1;
                    }
                },
                |render, mut column, mut c, attr| {
                    let mut style = Style::default();
                    if attr.is_underline() {
                        style = style.underlined();
                    }
                    if attr.is_bold() {
                        style = style.bold();
                    }
                    if attr.is_reverse() {
                        style = style.reversed();
                    }

                    if render.row_flags.double_width {
                        column *= 2;
                    }

                    if let Some(cell) = buf.cell_mut((
                        area.left() + column as u16,
                        area.top() + render.row_idx as u16,
                    )) {
                        if render.row_flags.status_row && attr.is_upper_bit() {
                            c = c | 0x800;
                        }
                        cell.set_char(unicode::map_char(c).unwrap_or('.'));
                        cell.set_style(style);

                        if render.row_flags.double_width {
                            if let Some(cell) = buf.cell_mut((
                                area.left() + column as u16 + 1,
                                area.top() + render.row_idx as u16,
                            )) {
                                cell.set_char(' ');
                                cell.set_style(style);
                            }
                        }
                    }
                },
                render,
            );
            return;
        }

        let Some(rows) = self.mapper.row_count(&vram) else {
            return;
        };

        for mut row_idx in 0..=rows as u16 {
            let row = ((vram[vram_base + row_idx as usize * 2] as u16) >> 1) << 8;
            if row == 0 {
                continue;
            }
            // Handle smooth scrolling by chopping the top row
            if self.mapper.get(2) != 0 {
                if row_idx as u8 == self.mapper.get(0) {
                    continue;
                }
                if row_idx as u8 > self.mapper.get(0) {
                    row_idx -= 1;
                }
            }
            // Bit 2: double width
            // Bit 1: swap between screen 0 and screen 1 attributes
            let row_attrs = vram[vram_base + row_idx as usize * 2 + 1];
            let is_double_width = (row_attrs >> 2) & 3 != 0;
            // If true, force 132 characters per line
            let row_is_132 = vram[vram_base + row_idx as usize * 2] & 1 != 0;

            // Decode 12-bit character codes from packed 3-byte sequences
            let mut b = 0;
            let mut j = 0;

            // First segment: 72 chars, bytes 0-107
            for i in 0..108 {
                let char = vram[row as usize + i];
                match i % 3 {
                    0 => b = char as u16,
                    1 => {
                        b |= ((char & 0xf) as u16) << 8;
                        line[j] = b;
                        j += 1;
                        b = ((char & 0xf0) as u16) >> 4;
                    }
                    _ => {
                        b |= (char as u16) << 4;
                        line[j] = b;
                        j += 1;
                    }
                }
            }
            // Second segment: bytes 128-220
            for i in 128..221 {
                let char = vram[row as usize + i];
                let i = i + 1;
                match i % 3 {
                    0 => b = char as u16,
                    1 => {
                        b |= ((char & 0xf) as u16) << 8;
                        line[j] = b;
                        j += 1;
                        b = ((char & 0xf0) as u16) >> 4;
                    }
                    _ => {
                        b |= (char as u16) << 4;
                        line[j] = b;
                        j += 1;
                    }
                }
            }

            // Extract attributes
            for i in 1..133 {
                let bit = ((i % 4) * 2) as u8;
                attr[i - 1] = (vram[row as usize + 0xdd + (i / 4)] >> bit) & 0x3;
                let cell_attr = ((line[i - 1] & 0xf00) >> 8) as u8;
                attr[i - 1] |= cell_attr << 2;
            }

            // Render the line
            match self.display_mode {
                DisplayMode::Bytes => {
                    let row_header = format!("{:02X}|", row >> 8);
                    let mut col = 0;
                    for (i, b) in vram[row as usize..row as usize + 256].iter().enumerate() {
                        if col < area.width {
                            let hex_str = format!("{:02X}", b);
                            for ch in hex_str.chars() {
                                if let Some(cell) =
                                    buf.cell_mut((area.left() + col, area.top() + row_idx))
                                {
                                    cell.set_symbol(&ch.to_string());
                                    let mut style = if i % 2 == 0 {
                                        Style::default()
                                    } else {
                                        Style::default().bold()
                                    };
                                    if i > 107 && i < 128 {
                                        style = style.fg(Color::Blue);
                                    }
                                    if i > 221 {
                                        style = style.fg(Color::Red);
                                    }
                                    cell.set_style(style);
                                }
                                col += 1;
                            }
                        }
                    }
                }
                DisplayMode::NibbleTriplet => {
                    let row_header = format!(
                        "{:02X}{:02X}|",
                        vram[vram_base + row_idx as usize * 2],
                        vram[vram_base + row_idx as usize * 2 + 1]
                    );
                    let mut col = 0;
                    for ch in row_header.chars() {
                        if col < area.width {
                            if let Some(cell) =
                                buf.cell_mut((area.left() + col, area.top() + row_idx))
                            {
                                cell.set_symbol(&ch.to_string());
                                cell.set_style(Style::default());
                            }
                            col += 1;
                        }
                    }
                    for (i, char_code) in line.iter().take(132).enumerate() {
                        let hex_str = format!("{:03X}", char_code);
                        for ch in hex_str.chars() {
                            if col < area.width {
                                if let Some(cell) =
                                    buf.cell_mut((area.left() + col, area.top() + row_idx))
                                {
                                    cell.set_symbol(&ch.to_string());
                                    cell.set_style(if i % 2 == 0 {
                                        Style::default()
                                    } else {
                                        Style::default().bold()
                                    });
                                }
                                col += 1;
                            }
                        }
                    }
                }
                DisplayMode::Normal => {}
            }
        }
    }
}

pub fn run(
    system: System,
    cpu: Cpu,
    debugger: Option<Debugger>,
    show_mapper: bool,
    show_vram: bool,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(io::stdout(), crossterm::terminal::EnterAlternateScreen,)?;
    crossterm::execute!(
        io::stdout(),
        crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
    )?;

    let res = run_inner(system, cpu, debugger, show_mapper, show_vram)?;

    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(io::stdout(), crossterm::terminal::LeaveAlternateScreen,)?;
    Ok(res)
}

fn run_inner(
    mut system: System,
    mut cpu: Cpu,
    debugger: Option<Debugger>,
    show_mapper: bool,
    show_vram: bool,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    let mut running = true;
    let mut hex = DisplayMode::Normal;
    let mut pc_trace = false;
    let mut keyboard = CrosstermKeyboard::default();
    let mut terminal = ratatui::Terminal::new(CrosstermBackend::new(io::stdout()))?;
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
        }

        if system.instruction_count % 0x1000 == 0 || !running {
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
                            let difference = system.pc_bitset.difference(&system.pc_bitset_current);
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

            let vram = &system.memory.vram[system.memory.mapper.vram_offset_display() as usize..];
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

                    if show_mapper {
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
                                Span::styled(format!("{:02X} ", system.memory.mapper.get(i)), style)
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

                    if show_vram {
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
    Ok(system.instruction_count)
}
