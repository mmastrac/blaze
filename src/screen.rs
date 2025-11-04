use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style, Stylize},
    widgets::Widget,
};

pub struct Screen<'a> {
    vram: &'a [u8],
    display_mode: DisplayMode,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DisplayMode {
    Normal,
    NibbleTriplet,
    Bytes,
}

impl<'a> Screen<'a> {
    pub fn new(vram: &'a [u8]) -> Self {
        Self {
            vram,
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
        let vram_base = (vram[0x73] as usize) << 8;
        let hex = self.display_mode != DisplayMode::Normal;

        let mut line = [0_u16; 256];
        let mut attr = [0_u8; 256];

        for row_idx in 0..area.height.min(48) {
            let row = ((vram[vram_base + row_idx as usize * 2] as u16) >> 1) << 8;
            // Bit 2: double width
            // Bit 1: swap between screen 0 and screen 1 attributes
            let row_attrs = vram[vram_base + row_idx as usize * 2 + 1];
            let is_double_width = row_attrs & (1 << 2) != 0;
            // If true, force 132 characters per line
            let row_is_132 = vram[vram_base + row_idx as usize * 2] & 1 != 0;

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
                    for char_code in line.iter().take(132) {
                        let hex_str = format!("{:03X}", char_code);
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
                }
                DisplayMode::Normal => {
                    // Character mode: show row number and characters
                    let row_header = format!("{:02X}|", row >> 8);

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

                    // Render characters
                    for i in 0..132.min((area.width - col) as usize) {
                        let char_code = line[i] & 0xff;
                        let ch = if char_code == 0 || char_code == 0x98 {
                            ' '
                        } else if char_code < 0x20 || char_code > 0x7e {
                            '.'
                        } else {
                            char::from(char_code as u8)
                        };

                        if let Some(cell) = buf.cell_mut((area.left() + col, area.top() + row_idx))
                        {
                            if char_code == 0 && attr[i] >> 2 == 0xe {
                                cell.set_symbol(" ");
                                cell.set_style(Style::default());
                                col += 1;
                                continue;
                            }
                            cell.set_symbol(&ch.to_string());
                            let mut style = Style::default();
                            if attr[i] & 1 != 0 {
                                style = style.underlined();
                            }
                            if attr[i] & 2 != 0 {
                                style = style.bold();
                            }
                            if attr[i] & 8 != 0 {
                                style = style.bold();
                            }
                            if attr[i] & 16 != 0 {
                                style = style.reversed();
                            }
                            if attr[i] & 32 != 0 {
                                style = style.rapid_blink();
                            }
                            cell.set_style(style);
                        }
                        col += 1;
                        if is_double_width {
                            if let Some(cell) =
                                buf.cell_mut((area.left() + col, area.top() + row_idx))
                            {
                                cell.set_symbol(" ");
                                cell.set_style(Style::default());
                            }
                            col += 1;
                        }
                    }
                }
            }
        }
    }
}
