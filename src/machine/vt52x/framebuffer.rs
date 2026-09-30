use crate::machine::generic::color::DEFAULT_COLOR;
use crate::machine::generic::display::{Display, FRAME_WIDTH, LineSize, TextAttr, TextLine};
use crate::machine::generic::glyphs::GlyphTable;
use crate::machine::vt52x::System;
use crate::machine::vt52x::memory::{
    ATTR_BLINK, ATTR_BOLD, ATTR_INVISIBLE, ATTR_REVERSE, ATTR_UNDERLINE, LOW_XDATA_BASE, RAM,
    RENDITION_BOTTOM_HALF, RENDITION_DOUBLE_HEIGHT, RENDITION_DOUBLE_WIDTH, TEXT_ROWS, TextCell,
};
use std::sync::OnceLock;

/// Glyph fingerprints for the VT510, VT520 and VT525 fonts in every screen
/// format: `glyphs.txt` from the ignored `glyph_table` test, then
/// `glyphs-vision.txt` for glyphs no character set reaches, identified from
/// images.
pub(crate) fn glyphs() -> &'static GlyphTable {
    static TABLE: OnceLock<GlyphTable> = OnceLock::new();
    TABLE.get_or_init(|| {
        GlyphTable::parse(&[
            include_str!("glyphs.txt"),
            include_str!("glyphs-vision.txt"),
        ])
    })
}

/// The pixel lines of a cell's glyph, as the display reads them.
pub(crate) fn glyph_lines(memory: &RAM, cell: &TextCell, row_lines: usize) -> Vec<u16> {
    (0..row_lines)
        .map(|line| memory.glyph_line(cell, line))
        .collect()
}

/// Calls `f(display row, column, cell, scan lines per row)` for each cell on
/// the screen.
pub(crate) fn for_each_cell(memory: &RAM, mut f: impl FnMut(usize, usize, &TextCell, usize)) {
    for row in display_rows(memory) {
        let cells = &memory.screen_row(row.line / row.row_lines % TEXT_ROWS)[..row.columns()];
        for (column, c) in cells.iter().enumerate() {
            f(row.row, column, c, row.row_lines);
        }
    }
}

struct DisplayRow {
    row: usize,
    line: usize,
    row_lines: usize,
    format: u8,
    /// 0x7FBA or 0x7FBE: bit 0 is set in WYSE modes, where attribute bit 7
    /// means dim rather than bold.
    attributes: u8,
}

impl DisplayRow {
    fn dim(&self, cell: &TextCell) -> bool {
        self.attributes & 1 != 0 && cell.attr & ATTR_BOLD != 0
    }

    fn reversed(&self) -> bool {
        self.format & 0x40 != 0
    }

    fn columns(&self) -> usize {
        if self.format & 0x10 != 0 { 132 } else { 80 }
    }
}

/// The line table the display reads: 0x7E00, or 0x7E80 when 0x7FB1 bit 1
/// is set. Entries 0x00-0x3F point each display row at a stored row;
/// 0x40-0x7F are the row flags.
fn line_table(memory: &RAM) -> usize {
    // The VT510 keeps its line tables at 0x0000 and 0x0080; what selects
    // between them is not known yet, and the firmware keeps both alike.
    if memory.model.is_vt51x() {
        return 0x0000;
    }
    if memory.regs.regs[0xB1] & 0x02 != 0 {
        0x7E80
    } else {
        0x7E00
    }
}

/// The display walks the line table: each entry points a display row at a
/// stored row (in pairs of scan lines, like 0x7FC4), so scrolled text comes
/// out in screen order. Flag bit 7 puts a row in window 2, which takes its
/// row height and reverse screen from 0x7FBC instead of 0x7FB8.
fn display_rows(memory: &RAM) -> impl Iterator<Item = DisplayRow> + '_ {
    let table = LOW_XDATA_BASE + line_table(memory);
    // The table ends after its last non-zero entry (row 0 may be 0).
    let rows = (1..0x40)
        .rev()
        .find(|&r| memory.dram[table + r] != 0)
        .map_or(0, |r| r + 1);
    (0..rows).map(move |row| {
        let lower = memory.dram[table + 0x40 + row] & 0x80 != 0;
        DisplayRow {
            row,
            line: memory.dram[table + row] as usize * 2,
            row_lines: memory.row_lines(lower),
            format: memory.window_format(lower),
            attributes: memory.window_attributes(lower),
        }
    })
}

impl Display for System {
    fn render_framebuffer(&self, frame: &mut [u8]) {
        let memory = &self.memory;
        let height = frame.len() / (FRAME_WIDTH * 4);
        let ramdac = &memory.regs.ramdac;
        let color = |on: bool, colour: u8, dim: bool, bold: bool| {
            let c = if ramdac.loaded {
                let index = if on { colour >> 4 } else { colour & 0x0F };
                ramdac.rgb(if dim && on { index | 0x10 } else { index })
            } else if on && bold {
                DEFAULT_COLOR.bold
            } else if on && dim {
                let (r, g, b) = DEFAULT_COLOR.foreground;
                (r / 3 * 2, g / 3 * 2, b / 3 * 2)
            } else if on {
                DEFAULT_COLOR.foreground
            } else {
                DEFAULT_COLOR.background
            };
            [c.0, c.1, c.2, 0xff]
        };
        for pixel in frame.chunks_exact_mut(4) {
            pixel.copy_from_slice(&color(false, 0, false, false));
        }
        if memory.display_blanked() {
            return;
        }
        let blink_off = memory.regs.regs[0xB1] & 0x08 != 0;
        let mut y = 0;
        for row in display_rows(memory) {
            let width = if row.columns() == 132 { 6 } else { 10 };
            let cells = memory.screen_row(row.line / row.row_lines % TEXT_ROWS);
            for line in 0..row.row_lines {
                if y >= height {
                    return;
                }
                let out = &mut frame[y * FRAME_WIDTH * 4..(y + 1) * FRAME_WIDTH * 4];
                for (column, cell) in cells[..row.columns()].iter().enumerate() {
                    let dim = row.dim(cell);
                    let bold = cell.attr & ATTR_BOLD != 0 && !dim;
                    let bits = cell_pixels(memory, cell, line, row.row_lines, blink_off);
                    for x in 0..width {
                        let offset = (column * width + x) * 4;
                        if offset >= out.len() {
                            break;
                        }
                        let on = (bits >> x & 1 != 0) ^ row.reversed();
                        out[offset..offset + 4].copy_from_slice(&color(on, cell.colour, dim, bold));
                    }
                }
                y += 1;
            }
        }
    }

    /// Screen reverse flips each cell's reverse attribute. A blanked display
    /// shows no rows.
    fn render_textbuffer(
        &self,
        line: &mut dyn FnMut(usize, TextLine),
        cell: &mut dyn FnMut(usize, usize, char, TextAttr),
    ) {
        let memory = &self.memory;
        if memory.display_blanked() {
            return;
        }
        for row in display_rows(memory) {
            let cells = &memory.screen_row(row.line / row.row_lines % TEXT_ROWS)[..row.columns()];
            line(
                row.row,
                TextLine {
                    size: LineSize::Single,
                    columns: row.columns(),
                    reverse: row.reversed(),
                },
            );
            for (column, c) in cells.iter().enumerate() {
                let mut attr = TextAttr::NONE;
                if c.attr & ATTR_BOLD != 0 {
                    attr |= TextAttr::BOLD;
                }
                if c.attr & ATTR_UNDERLINE != 0 {
                    attr |= TextAttr::UNDERLINE;
                }
                if c.attr & ATTR_BLINK != 0 {
                    attr |= TextAttr::BLINK;
                }
                if (c.attr & ATTR_REVERSE != 0) ^ row.reversed() {
                    attr |= TextAttr::REVERSE;
                }
                if c.rendition & RENDITION_DOUBLE_WIDTH != 0 {
                    attr |= if c.continuation {
                        TextAttr::RIGHT_HALF
                    } else {
                        TextAttr::LEFT_HALF
                    };
                }
                if c.rendition & RENDITION_DOUBLE_HEIGHT != 0 {
                    attr |= if c.rendition & RENDITION_BOTTOM_HALF != 0 {
                        TextAttr::BOTTOM_HALF
                    } else {
                        TextAttr::TOP_HALF
                    };
                }
                // Attribute bits 1-0 = 3 mark a soft (DECDLD) font cell.
                let ch = if c.attr & ATTR_INVISIBLE != 0 {
                    ' '
                } else if c.attr & 3 == 3 {
                    '▒'
                } else {
                    glyphs()
                        .get_or_blank(&glyph_lines(memory, c, row.row_lines), c.attr & 3, c.ch)
                        .unwrap_or('·')
                };
                cell(row.row, column, ch, attr);
            }
        }
    }
}

fn cell_pixels(
    memory: &RAM,
    cell: &TextCell,
    line: usize,
    row_lines: usize,
    blink_off: bool,
) -> u16 {
    let double_height = cell.rendition & RENDITION_DOUBLE_HEIGHT != 0;
    let bottom_half = cell.rendition & RENDITION_BOTTOM_HALF != 0;
    let src = match (double_height, bottom_half) {
        (true, false) => line / 2,
        (true, true) => row_lines / 2 + line / 2,
        (false, _) => line,
    };
    let mut bits = memory.glyph_line(cell, src);
    if cell.attr & ATTR_INVISIBLE != 0 || (cell.attr & ATTR_BLINK != 0 && blink_off) {
        bits = 0;
    }
    if cell.attr & ATTR_UNDERLINE != 0 && src == cell.underline_line as usize {
        bits = CELL_MASK;
    }
    if cell.attr & ATTR_REVERSE != 0 {
        bits = !bits & CELL_MASK;
    }
    // Double width spreads the cell's 10 pixels over two cells.
    if cell.rendition & RENDITION_DOUBLE_WIDTH != 0 {
        bits = double_pixels(bits, cell.continuation as usize);
    }
    bits
}

/// The 10 pixels of one framebuffer cell.
const CELL_MASK: u16 = 0x3FF;

/// Half `half` (0 = left) of a cell's pixels doubled to fill a whole cell.
fn double_pixels(bits: u16, half: usize) -> u16 {
    let mut out = 0;
    for i in 0..10 {
        if bits >> (half * 5 + i / 2) & 1 != 0 {
            out |= 1 << i;
        }
    }
    out
}
