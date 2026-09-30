use crate::machine::generic::color::DEFAULT_COLOR;
use crate::machine::generic::display::{Display, FRAME_WIDTH, LineSize, TextAttr, TextLine};
use crate::machine::vt420::System;
use crate::machine::vt420::unicode;
use crate::machine::vt420::video::{RowFlags, decode_font, decode_font_downloadable, decode_vram};

impl Display for System {
    fn render_framebuffer(&self, frame: &mut [u8]) {
        #[derive(Default)]
        struct Render<'a> {
            row: usize,
            row_offset: usize,
            row_flags: RowFlags,
            start_row: usize,
            frame: &'a mut [u8],
            chargen_disabled: bool,
            smooth: (u8, u8, u8),
        }
        let render = Render {
            smooth: (
                self.memory.display_mapper.get(0),
                self.memory.display_mapper.get(1),
                self.memory.display_mapper.get(2),
            ),
            frame,
            chargen_disabled: self.memory.display_mapper.disable_chargen(),
            ..Default::default()
        };
        let mut font = [0_u16; 16];
        decode_vram(
            &self.memory.vram[self.memory.display_mapper.vram_offset_display() as usize..],
            &self.memory.display_mapper,
            |render, row, _attr, row_flags| {
                render.row += render.row_flags.row_height as usize;
                render.row_offset += 800 * 4 * render.row_flags.row_height as usize;
                render.row_flags = row_flags;

                render.start_row = 0;

                if render.row_flags.status_row {
                    render.row = 400;
                    render.row_offset = 800 * 4 * render.row;
                } else if render.smooth.2 != 0 && (render.smooth.0..=render.smooth.1).contains(&row)
                {
                    if row == render.smooth.0 {
                        render.start_row = render.smooth.2 as usize;
                        render.row_flags.row_height -= render.smooth.2;
                    } else if row == render.smooth.1 {
                        render.row_flags.row_height = render.smooth.2;
                    }
                }
            },
            |render, column, c, attr| {
                let c = c as usize;
                let mut c = c * 2;
                // The status bar rendering rules are strange...  if bit 0x8 in the main attribute
                // nibble is not set, we use the normal 132-column font, otherwise we use this as
                // "direct pointer" to an extended char.
                if render.row_flags.status_row && !attr.is_upper_bit() {
                    c = c.saturating_add(1);
                }

                let color = if render.chargen_disabled && !render.row_flags.status_row {
                    [DEFAULT_COLOR.background, DEFAULT_COLOR.background]
                } else {
                    let mut pos = if attr.is_bold() {
                        DEFAULT_COLOR.bold
                    } else {
                        DEFAULT_COLOR.foreground
                    };
                    let neg = DEFAULT_COLOR.background;

                    if !render.row_flags.status_row && attr.is_upper_bit() {
                        pos = if self.memory.display_mapper.is_blink() {
                            if attr.is_bold() {
                                DEFAULT_COLOR.foreground
                            } else {
                                DEFAULT_COLOR.background
                            }
                        } else if attr.is_bold() {
                            DEFAULT_COLOR.bold
                        } else {
                            DEFAULT_COLOR.foreground
                        }
                    };

                    if render.row_flags.invert ^ attr.is_reverse() {
                        [pos, neg]
                    } else {
                        [neg, pos]
                    }
                };

                // Downloadable fonts follow a different rendering rule
                if c / 2 >= 0x1A0 {
                    // Custom font data
                    let font_address_base =
                        c * 16 + 0x8000 + (!render.row_flags.is_80 as usize) * 0x4000;
                    decode_font_downloadable(
                        self.memory.vram.as_ref(),
                        (c / 2) as u16,
                        render.row_flags.screen_2,
                        font_address_base as _,
                        render.row_flags.is_80,
                        &mut font,
                    );
                } else {
                    let font_address_base = c * 16 + 0x8000 + render.row_flags.font as usize;
                    decode_font(
                        self.memory.vram.as_ref(),
                        font_address_base as _,
                        render.row_flags.is_80,
                        &mut font,
                    );
                };

                let width = if render.row_flags.is_80 { 10 } else { 6 };
                let mut offset = render.row_offset;
                for mut y in 0..render.row_flags.row_height as usize {
                    if render.row + y >= FRAME_WIDTH {
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
                            if attr.is_underline() && y == render.row_flags.row_height as usize - 1
                            {
                                pixel = true;
                            }
                            let color = color[pixel as usize];
                            render.frame[offset + x_offset] = color.0;
                            render.frame[offset + x_offset + 1] = color.1;
                            render.frame[offset + x_offset + 2] = color.2;
                            render.frame[offset + x_offset + 3] = 0xff;
                            render.frame[offset + x_offset + 4] = color.0;
                            render.frame[offset + x_offset + 5] = color.1;
                            render.frame[offset + x_offset + 6] = color.2;
                            render.frame[offset + x_offset + 7] = 0xff;
                        }
                    } else {
                        for x in 0..width {
                            let x_offset = (column as usize * width + x) * 4;
                            let mut pixel = font[y + render.start_row] & (1 << x) != 0;
                            if attr.is_underline() && y == render.row_flags.row_height as usize - 1
                            {
                                pixel = true;
                            }
                            let color = color[pixel as usize];
                            render.frame[offset + x_offset] = color.0;
                            render.frame[offset + x_offset + 1] = color.1;
                            render.frame[offset + x_offset + 2] = color.2;
                            render.frame[offset + x_offset + 3] = 0xff;
                        }
                    }
                    offset += 800 * 4;
                }
            },
            render,
        );

        // Stopgap to fix the leftover pixels at the end of the frame
        // if render.row_offset < render.frame.len() {
        //     render.frame[render.row_offset..].fill(0);
        // }
    }

    fn render_textbuffer(
        &self,
        line: &mut dyn FnMut(usize, TextLine),
        cell: &mut dyn FnMut(usize, usize, char, TextAttr),
    ) {
        let vram = &self.memory.vram[self.memory.display_mapper.vram_offset_display() as usize..];
        let mapper = &self.memory.display_mapper;

        #[derive(Default)]
        struct Render {
            row_idx: usize,
            row_flags: RowFlags,
            smooth_row: u8,
        }

        let render = Render {
            smooth_row: if mapper.get(2) != 0 {
                mapper.get(0)
            } else {
                u8::MAX
            },
            ..Default::default()
        };

        decode_vram(
            vram,
            mapper,
            |render, row, _attr, row_flags| {
                render.row_idx = row as usize;
                render.row_flags = row_flags;

                if row >= render.smooth_row {
                    render.row_idx = render.row_idx.saturating_sub(1);
                }

                let size = if render.row_flags.double_height_top {
                    LineSize::DoubleHeightTop
                } else if render.row_flags.double_height_bottom {
                    LineSize::DoubleHeightBottom
                } else if render.row_flags.double_width {
                    LineSize::DoubleWidth
                } else {
                    LineSize::Single
                };
                line(
                    render.row_idx,
                    TextLine {
                        size,
                        columns: if render.row_flags.is_80 { 80 } else { 132 },
                        reverse: render.row_flags.invert,
                    },
                );
            },
            |render, column, mut c, attr| {
                let mut style = TextAttr::NONE;
                if attr.is_underline() {
                    style |= TextAttr::UNDERLINE;
                }
                if attr.is_bold() {
                    style |= TextAttr::BOLD;
                }
                if attr.is_reverse() ^ render.row_flags.invert {
                    style |= TextAttr::REVERSE;
                }

                if render.row_flags.status_row && attr.is_upper_bit() {
                    c |= 0x800;
                }
                cell(
                    render.row_idx,
                    column as usize,
                    unicode::map_char(c).unwrap_or('.'),
                    style,
                );
            },
            render,
        );
    }
}
