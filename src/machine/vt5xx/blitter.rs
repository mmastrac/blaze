use tracing::{info, trace, warn};

use crate::machine::vt52x::memory::{
    DRAM_SIZE, RAM, REG_BLIT_CMD, Registers, TEXT_COLS, TEXT_ROWS, TextCell, vt51x, vt52x,
};

pub const RENDITION_DOUBLE_HEIGHT: u8 = 0x01;
pub const RENDITION_DOUBLE_WIDTH: u8 = 0x02;
pub const RENDITION_BOTTOM_HALF: u8 = 0x04;

fn rect_size(r: &[u8; 256]) -> (usize, usize) {
    let lines = (!r[0xC7]) as usize + 1;
    let width = (0x100 - r[0xC6] as usize) & 0xFF;
    (lines, width)
}

const REG_VIDEO_FORMAT: u8 = 0xB2;

const VT51X_BUSY_INSTRUCTIONS: usize = 8;

fn vt51x_command(value: u8) -> u8 {
    if value & 0x0C == 0x08 {
        value | 0x04
    } else {
        value
    }
}

impl Registers {
    pub(crate) fn blit_reg_write(&mut self, reg: u8, value: u8, pc: u32) {
        if (0xC0..=0xCF).contains(&reg) {
            trace!("blitreg {reg:02X}={value:02X} @ {pc:05X}");
        }
        if reg == REG_BLIT_CMD {
            self.blit_count += 1;
            let r = &self.regs;
            trace!(
                "blit cmd={value:02X} src={:02X}{:02X}{:02X} dst={:02X}{:02X}{:02X} count={:02X}{:02X} 4D={:02X} 4E={:02X} @ {pc:05X}",
                r[0xC2],
                r[0xC1],
                r[0xC0],
                r[0xC5],
                r[0xC4],
                r[0xC3],
                r[0xC7],
                r[0xC6],
                r[0xCD],
                r[0xCE]
            );
        }
    }
}

impl RAM {
    pub(crate) fn blit_write(&mut self, value: u8, pc: u32) {
        let value = if self.model.is_vt51x() {
            vt51x_command(value)
        } else {
            value
        };
        self.regs.write(REG_BLIT_CMD, value, pc);
        if self.model.is_vt51x() {
            self.regs.blit_busy = VT51X_BUSY_INSTRUCTIONS;
        }
        self.blit(value, pc);
    }

    fn blit(&mut self, cmd: u8, pc: u32) {
        let r = &self.regs.regs;
        let addr = |lo: u8, mid: u8, hi: u8| {
            (lo as usize & 0x7F) | (mid as usize) << 7 | (hi as usize) << 15
        };
        let src = addr(r[0xC0], r[0xC1], r[0xC2]);
        let dst = addr(r[0xC3], r[0xC4], r[0xC5]);
        let count = 0x10000 - (r[0xC6] as usize | (r[0xC7] as usize) << 8);
        let lower = r[REG_VIDEO_FORMAT as usize] & 1 != 0;
        match cmd {
            0x13 => {
                for i in 0..count {
                    self.dram[(dst + i) % DRAM_SIZE] = self.dram[(src + i) % DRAM_SIZE];
                }
            }
            0x03 => {
                for i in 0..count {
                    self.dram[(dst + DRAM_SIZE - i) % DRAM_SIZE] =
                        self.dram[(src + DRAM_SIZE - i) % DRAM_SIZE];
                }
            }
            0x9D | 0x8D => {
                let (_, width) = rect_size(r);
                let row_lines = self.row_lines(lower);
                let (sx, sy) = (r[0xC0] as usize, r[0xC1] as usize | (r[0xC2] as usize) << 8);
                let (dx, dy) = (r[0xC3] as usize, r[0xC4] as usize * 2);
                let descending = cmd == 0x8D;
                let columns: Vec<(usize, usize)> = (0..width)
                    .map(|i| {
                        if descending {
                            (sx.wrapping_sub(i), dx.wrapping_sub(i))
                        } else {
                            (sx + i, dx + i)
                        }
                    })
                    .collect();
                let (s_row, d_row) = (sy / row_lines % TEXT_ROWS, dy / row_lines % TEXT_ROWS);
                for &(s_col, d_col) in &columns {
                    let from = s_row * TEXT_COLS + s_col % TEXT_COLS;
                    let to = d_row * TEXT_COLS + d_col % TEXT_COLS;
                    self.text[to] = self.text[from];
                }
            }
            0x9E => {
                let (lines, width) = rect_size(r);
                let row_lines = self.row_lines(lower);
                let (x, y) = (r[0xC3] as usize, r[0xC4] as usize * 2);
                if r[0xCD] != 0 {
                    warn!("0x9E fill with {:02X} @ {pc:05X}", r[0xCD]);
                }
                for line in (y..y + lines).step_by(row_lines) {
                    for i in 0..width {
                        let cell = (line / row_lines % TEXT_ROWS) * TEXT_COLS + (x + i) % TEXT_COLS;
                        self.text[cell] = TextCell::default();
                    }
                }
            }
            0x9C | 0xBC | 0xDC | 0xFC => {
                let (_, width) = rect_size(r);
                let right_half = cmd & 0x40 != 0;
                let row_lines = self.row_lines(lower);
                let soft = cmd & 0x20 != 0;
                let soft_glyph = soft.then_some(src as u32);
                let (x, y) = (r[0xC3] as usize, r[0xC4] as usize * 2);
                let rendition = r[0xCE];
                let double_width = rendition & RENDITION_DOUBLE_WIDTH != 0;
                let cell = TextCell {
                    ch: r[0xC0],
                    attr: if soft { r[0xCD] } else { r[0xC1] | r[0xCD] },
                    continuation: right_half && double_width,
                    soft_glyph,
                    rendition,
                    underline_line: if self.model.is_vt51x() {
                        vt51x::underline_line(row_lines)
                    } else {
                        r[REG_VIDEO_FORMAT as usize] >> 4
                    },
                    colour: r[0xCF],
                    font_column: vt52x::font_column(
                        r[REG_VIDEO_FORMAT as usize],
                        r[0xC1] | r[0xCD],
                    ),
                };
                let row = (y / row_lines % TEXT_ROWS) * TEXT_COLS;
                for i in 0..width {
                    self.text[row + (x + i) % TEXT_COLS] = TextCell {
                        continuation: cell.continuation || i > 0,
                        ..cell
                    };
                }
            }
            _ => {
                let n = self.regs.unknown_blits.entry(cmd).or_insert(0);
                if *n == 0 {
                    info!(
                        "unhandled blit cmd {cmd:02X} src={src:06X} dst={dst:06X} count={count:X} @ {pc:05X}"
                    );
                }
                *n += 1;
            }
        }
    }
}
