use std::{cell::RefCell, io, rc::Rc};

use ratatui::{
    backend::{ClearType, WindowSize},
    buffer::Cell,
    layout::{Position, Size},
};
use ssu::session::loopback::WakeableQueueSend;

pub const VT_DOUBLE_WIDTH_LINE: ratatui::style::Color = ratatui::style::Color::Rgb(1, 1, 1);
pub const VT_DOUBLE_HEIGHT_TOP_LINE: ratatui::style::Color = ratatui::style::Color::Rgb(1, 1, 2);
pub const VT_DOUBLE_HEIGHT_BOTTOM_LINE: ratatui::style::Color = ratatui::style::Color::Rgb(1, 1, 3);

#[derive(Clone)]
pub struct DecBackend {
    pub pending: WakeableQueueSend,
    pub size: Rc<RefCell<Size>>,
    pub cursor_pos: Rc<RefCell<Position>>,
    pub current_style: Rc<RefCell<ratatui::style::Style>>,
    pub cursor_visible: Rc<RefCell<bool>>,
}

impl DecBackend {
    pub fn new(output: WakeableQueueSend) -> Self {
        Self {
            pending: output,
            size: Rc::new(RefCell::new(Size::new(80, 24))),
            cursor_pos: Rc::new(RefCell::new(Position::new(0, 0))),
            current_style: Rc::new(RefCell::new(ratatui::style::Style::default())),
            cursor_visible: Rc::new(RefCell::new(true)),
        }
    }

    pub fn send_bytes(&self, buf: &[u8]) {
        self.pending.send_bytes(buf);
    }
}

impl io::Write for DecBackend {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.pending.send_bytes(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl DecBackend {
    fn write_bytes(&self, bytes: &[u8]) {
        self.pending.send_bytes(bytes);
    }

    fn write_str(&self, s: &str) {
        // Treat UTF-8 chars as bytes
        for c in s.chars() {
            if (c as u32) < 0x80 {
                self.pending.send(c as u8);
            } else if (c as u32) < 0x100 {
                self.pending.send(b'\x0e');
                self.pending.send(c as u8 - 0x80);
                self.pending.send(b'\x0f');
            } else {
                self.pending.send(b'?');
            }
        }
    }

    fn write_csi(&self, params: &str, final_byte: u8) {
        self.write_bytes(b"\x1b[");
        self.write_str(params);
        self.write_bytes(&[final_byte]);
    }

    fn set_cursor_pos(&self, x: u16, y: u16) {
        let mut pos = self.cursor_pos.borrow_mut();
        if pos.x != x || pos.y != y {
            // VT420 uses 1-based indexing, and format is ESC [ row ; col H
            self.write_csi(&format!("{};{}", y + 1, x + 1), b'H');
            pos.x = x;
            pos.y = y;
        }
    }

    fn apply_style(&self, style: &ratatui::style::Style) {
        let mut current = self.current_style.borrow_mut();
        if *current == *style {
            return;
        }

        // Build SGR (Select Graphic Rendition) sequence
        let mut codes = Vec::new();

        // Reset first
        codes.push(0);

        // Text modifiers
        if style.add_modifier.contains(ratatui::style::Modifier::BOLD) {
            codes.push(1);
        }
        if style.add_modifier.contains(ratatui::style::Modifier::DIM) {
            codes.push(2);
        }
        if style
            .add_modifier
            .contains(ratatui::style::Modifier::UNDERLINED)
        {
            codes.push(4);
        }
        if style
            .add_modifier
            .contains(ratatui::style::Modifier::SLOW_BLINK)
        {
            codes.push(5);
        }
        if style
            .add_modifier
            .contains(ratatui::style::Modifier::REVERSED)
        {
            codes.push(7);
        }
        if style
            .add_modifier
            .contains(ratatui::style::Modifier::HIDDEN)
        {
            codes.push(8);
        }

        // Remove modifiers
        if style.sub_modifier.contains(ratatui::style::Modifier::BOLD) {
            codes.push(22);
        }
        if style.sub_modifier.contains(ratatui::style::Modifier::DIM) {
            codes.push(22);
        }
        if style
            .sub_modifier
            .contains(ratatui::style::Modifier::UNDERLINED)
        {
            codes.push(24);
        }
        if style
            .sub_modifier
            .contains(ratatui::style::Modifier::SLOW_BLINK)
        {
            codes.push(25);
        }
        if style
            .sub_modifier
            .contains(ratatui::style::Modifier::RAPID_BLINK)
        {
            codes.push(25);
        }
        if style
            .sub_modifier
            .contains(ratatui::style::Modifier::REVERSED)
        {
            codes.push(27);
        }
        if style
            .sub_modifier
            .contains(ratatui::style::Modifier::HIDDEN)
        {
            codes.push(28);
        }

        // Write SGR sequence: ESC [ codes... m
        if !codes.is_empty() {
            let params: Vec<String> = codes.iter().map(|c| c.to_string()).collect();
            self.write_csi(&params.join(";"), b'm');
        }

        // Use a custom RGB color to indicate double width line
        if style.fg == Some(VT_DOUBLE_WIDTH_LINE) {
            self.write_str("\x1b#6");
        } else if style.fg == Some(VT_DOUBLE_HEIGHT_TOP_LINE) {
            self.write_str("\x1b#3");
        } else if style.fg == Some(VT_DOUBLE_HEIGHT_BOTTOM_LINE) {
            self.write_str("\x1b#4");
        } else if current.fg.is_some() {
            self.write_str("\x1b#5");
        }

        *current = *style;
    }
}

impl ratatui::backend::Backend for DecBackend {
    type Error = io::Error;

    fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        for (x, y, cell) in content {
            // Move cursor if needed
            self.set_cursor_pos(x, y);

            // Apply style if changed
            self.apply_style(&cell.style());

            // Write the symbol
            let symbol = cell.symbol();
            if !symbol.is_empty() {
                self.write_str(symbol);
                // Update cursor position after writing
                let mut pos = self.cursor_pos.borrow_mut();
                pos.x = x + 1;
            }
        }
        Ok(())
    }

    fn hide_cursor(&mut self) -> Result<(), Self::Error> {
        let mut visible = self.cursor_visible.borrow_mut();
        if *visible {
            // ESC [ ? 25 l - Hide cursor
            self.write_csi("?25", b'l');
            *visible = false;
        }
        Ok(())
    }

    fn show_cursor(&mut self) -> Result<(), Self::Error> {
        let mut visible = self.cursor_visible.borrow_mut();
        if !*visible {
            // ESC [ ? 25 h - Show cursor
            self.write_csi("?25", b'h');
            *visible = true;
        }
        Ok(())
    }

    fn get_cursor_position(&mut self) -> Result<Position, Self::Error> {
        Ok(*self.cursor_pos.borrow())
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> Result<(), Self::Error> {
        let pos = position.into();
        self.set_cursor_pos(pos.x, pos.y);
        Ok(())
    }

    fn clear(&mut self) -> Result<(), Self::Error> {
        // ESC [ 2 J - Clear entire screen
        self.write_csi("2", b'J');
        // Reset cursor to top-left
        self.set_cursor_pos(0, 0);
        // Reset style
        *self.current_style.borrow_mut() = ratatui::style::Style::default();
        self.write_csi("0", b'm');
        Ok(())
    }

    fn clear_region(&mut self, clear_type: ClearType) -> Result<(), Self::Error> {
        // VT420 clear operations
        match clear_type {
            ClearType::All => {
                // ESC [ 2 J - Clear entire screen
                self.write_csi("2", b'J');
            }
            ClearType::CurrentLine => {
                // ESC [ 2 K - Clear entire line
                self.write_csi("2", b'K');
            }
            ClearType::AfterCursor => {
                // ESC [ 0 J - Clear from cursor to end of screen
                self.write_csi("0", b'J');
            }
            ClearType::BeforeCursor => {
                // ESC [ 1 J - Clear from beginning to cursor
                self.write_csi("1", b'J');
            }
            ClearType::UntilNewLine => {
                // ESC [ 0 K - Clear from cursor to end of line
                self.write_csi("0", b'K');
            }
        }
        Ok(())
    }

    fn size(&self) -> Result<Size, Self::Error> {
        Ok(*self.size.borrow())
    }

    fn window_size(&mut self) -> Result<WindowSize, Self::Error> {
        Ok(WindowSize {
            columns_rows: *self.size.borrow(),
            pixels: Size::new(10, 16),
        })
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}
