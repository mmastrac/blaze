use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style, Stylize},
    widgets::Widget,
};

pub struct Screen<'a> {
    vram: &'a [u8],
    hex_mode: bool,
}

impl<'a> Screen<'a> {
    pub fn new(vram: &'a [u8]) -> Self {
        Self {
            vram,
            hex_mode: false,
        }
    }

    pub fn hex_mode(mut self, hex: bool) -> Self {
        self.hex_mode = hex;
        self
    }
}

impl<'a> Widget for Screen<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let vram = self.vram;
        let vram_base = (vram[0x73] as usize) << 8;
        let hex = self.hex_mode;

        let mut line = [0_u16; 256];
        let mut attr = [0_u8; 256];

        for row_idx in 0..area.height.min(48) {
            let row = ((vram[vram_base + row_idx as usize * 2] as u16) >> 1) << 8;

            // Decode 12-bit character codes from packed 3-byte sequences
            let mut b = 0;
            let mut j = 0;

            // First segment: bytes 0-107
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
            if hex {
                // Hex mode: show row header and hex values
                let row_header = format!(
                    "{:02X}{:02X}|",
                    vram[vram_base + row_idx as usize * 2],
                    vram[vram_base + row_idx as usize * 2 + 1]
                );

                let mut col = 0;
                for ch in row_header.chars() {
                    if col < area.width {
                        if let Some(cell) = buf.cell_mut((area.left() + col, area.top() + row_idx))
                        {
                            cell.set_symbol(&ch.to_string());
                            cell.set_style(Style::default());
                        }
                        col += 1;
                    }
                }

                // Show hex values for each character
                for char_code in line.iter().take(132) {
                    let hex_str = format!("{:03X} ", char_code);
                    for ch in hex_str.chars() {
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
                }
            } else {
                // Character mode: show row number and characters
                let row_header = format!("{:02X}|", row >> 8);

                let mut col = 0;
                for ch in row_header.chars() {
                    if col < area.width {
                        if let Some(cell) = buf.cell_mut((area.left() + col, area.top() + row_idx))
                        {
                            cell.set_symbol(&ch.to_string());
                            cell.set_style(Style::default());
                        }
                        col += 1;
                    }
                }

                // Render characters
                for i in 0..132.min((area.width - col) as usize) {
                    let char_code = line[i] & 0xff;
                    let ch = if char_code == 0 {
                        ' '
                    } else if char_code < 0x20 || char_code > 0x7e {
                        '.'
                    } else {
                        char::from(char_code as u8)
                    };

                    if let Some(cell) = buf.cell_mut((area.left() + col, area.top() + row_idx)) {
                        cell.set_symbol(&ch.to_string());
                        let mut style = Style::default();
                        if attr[i] & 1 != 0 {
                            style = style.underlined();
                        }
                        if attr[i] & 2 != 0 {
                            style = style.bold();
                        }
                        if attr[i] & 8 != 0 {
                            style = style.reversed();
                        }
                        cell.set_style(style);
                    }
                    col += 1;
                }
            }
        }
    }
}
