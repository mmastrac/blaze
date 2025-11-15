use std::{cell::RefCell, collections::VecDeque, io, rc::Rc, sync::mpsc};

use ratatui::{
    backend::{ClearType, WindowSize},
    buffer::Cell,
    layout::{Constraint, HorizontalAlignment, Position, Size},
    style::{Style, Stylize},
    symbols::border,
    text::{Line, Span},
    widgets::{Block, List, ListDirection, ListState, Padding, Paragraph, Wrap},
};
use tracing::trace;

const VT420_BORDER_SET: border::Set = border::Set {
    top_left: "|",
    top_right: "|",
    bottom_left: "+",
    bottom_right: "+",
    vertical_left: "|",
    vertical_right: "|",
    horizontal_top: " ",
    horizontal_bottom: "-",
};

const PAGE_MENU_ITEMS: [&str; 11] = [
    "Set 80 columns",
    "Set 132 columns",
    "", //
    "Set 24 rows",
    "Set 36 rows",
    "Set 48 rows",
    "", //
    "Page size 24",
    "Page size 36",
    "Page size 48",
    "Page size 72",
];

#[derive(Clone)]
struct Pending {
    pending: Rc<RefCell<VecDeque<u8>>>,
    size: Rc<RefCell<Size>>,
    cursor_pos: Rc<RefCell<Position>>,
    current_style: Rc<RefCell<ratatui::style::Style>>,
    cursor_visible: Rc<RefCell<bool>>,
}

impl Default for Pending {
    fn default() -> Self {
        Self {
            pending: Rc::new(RefCell::new(VecDeque::new())),
            size: Rc::new(RefCell::new(Size::new(80, 24))),
            cursor_pos: Rc::new(RefCell::new(Position::new(0, 0))),
            current_style: Rc::new(RefCell::new(ratatui::style::Style::default())),
            cursor_visible: Rc::new(RefCell::new(true)),
        }
    }
}

impl io::Write for Pending {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.pending.borrow_mut().extend(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Pending {
    fn write_bytes(&self, bytes: &[u8]) {
        self.pending.borrow_mut().extend(bytes);
    }

    fn write_str(&self, s: &str) {
        self.pending.borrow_mut().extend(s.as_bytes());
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
            .contains(ratatui::style::Modifier::ITALIC)
        {
            codes.push(3);
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
            .contains(ratatui::style::Modifier::RAPID_BLINK)
        {
            codes.push(6);
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
        if style
            .add_modifier
            .contains(ratatui::style::Modifier::CROSSED_OUT)
        {
            codes.push(9);
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
            .contains(ratatui::style::Modifier::ITALIC)
        {
            codes.push(23);
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
        if style
            .sub_modifier
            .contains(ratatui::style::Modifier::CROSSED_OUT)
        {
            codes.push(29);
        }

        // Write SGR sequence: ESC [ codes... m
        if !codes.is_empty() {
            let params: Vec<String> = codes.iter().map(|c| c.to_string()).collect();
            self.write_csi(&params.join(";"), b'm');
        }

        *current = *style;
    }
}

impl ratatui::backend::Backend for Pending {
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

pub struct DemoComm {
    tx: mpsc::SyncSender<u8>,
    rx: mpsc::Receiver<u8>,
    input_queue: vt_push_parser::VTPushParser,
    pending: Pending,
    xon: bool,
    input: bool,
    page: u8,

    screen: ratatui::Terminal<Pending>,
    list_state: ListState,
}

impl DemoComm {
    pub fn new(tx: mpsc::SyncSender<u8>, rx: mpsc::Receiver<u8>) -> Self {
        let mut pending = Pending::default();
        pending.size = Rc::new(RefCell::new(Size::new(80, 24)));
        let screen = ratatui::Terminal::new(pending.clone()).unwrap();
        Self {
            tx,
            rx,
            input_queue: vt_push_parser::VTPushParser::new(),
            screen,
            pending,
            xon: false,
            input: false,
            page: 0,
            list_state: ListState::default(),
        }
    }

    pub fn tick(&mut self) {
        loop {
            if let Ok(byte) = self.rx.try_recv() {
                if byte == 0x11 {
                    self.xon = true;
                    if self.pending.pending.borrow().is_empty() {
                        self.input = true;
                    }
                } else if byte == 0x13 {
                    self.xon = false;
                } else if byte == 0x0c {
                    // ctrl+L - clear screen
                    let screen = ratatui::Terminal::new(self.pending.clone()).unwrap();
                    self.screen = screen;
                    self.xon = true;
                    self.input = true;
                } else if byte == 0x0d {
                    self.input = true;
                    if self.page == 1 {
                        match self.list_state.selected() {
                            Some(0) => {
                                self.pending.pending.borrow_mut().extend(b"\x1b[80$|");
                            }
                            Some(1) => {
                                self.pending.pending.borrow_mut().extend(b"\x1b[132$|");
                            }
                            Some(3) => {
                                self.pending.pending.borrow_mut().extend(b"\x1b[24*|");
                            }
                            Some(4) => {
                                self.pending.pending.borrow_mut().extend(b"\x1b[36*|");
                            }
                            Some(5) => {
                                self.pending.pending.borrow_mut().extend(b"\x1b[48*|");
                            }
                            Some(7) => {
                                self.pending.pending.borrow_mut().extend(b"\x1b[24t");
                            }
                            Some(8) => {
                                self.pending.pending.borrow_mut().extend(b"\x1b[36t");
                            }
                            Some(9) => {
                                self.pending.pending.borrow_mut().extend(b"\x1b[48t");
                            }
                            Some(10) => {
                                self.pending.pending.borrow_mut().extend(b"\x1b[72t");
                            }
                            _ => (),
                        }
                    }
                } else {
                    self.input_queue.feed_with(
                        &[byte],
                        &mut |event: vt_push_parser::event::VTEvent<'_>| match event {
                            vt_push_parser::event::VTEvent::Csi(csi) => {
                                if csi.final_byte == b'w' && csi.intermediates.has(b'"') {
                                    if csi.params.len() == 5 {
                                        let width = csi.params.try_parse(1).unwrap_or(24_u16);
                                        let height = csi.params.try_parse(0).unwrap_or(80_u16);
                                        let left = csi.params.try_parse(2).unwrap_or(0_u16);
                                        let top = csi.params.try_parse(3).unwrap_or(0_u16);
                                        let page = csi.params.try_parse(4).unwrap_or(0_u16);

                                        let size = Size::new(width, height);
                                        if size != *self.pending.size.borrow() {
                                            *self.pending.size.borrow_mut() = size;
                                            self.input = true;
                                        }
                                    }
                                } else if csi.final_byte == b'C' {
                                    self.page = 1;
                                    self.input = true;
                                    self.list_state.select(Some(0));
                                } else if csi.final_byte == b'D' {
                                    self.page = 0;
                                    self.input = true;
                                } else if csi.final_byte == b'A' {
                                    self.list_state.select_previous();
                                    if PAGE_MENU_ITEMS
                                        .get(self.list_state.selected().unwrap_or_default())
                                        .cloned()
                                        .unwrap_or_default()
                                        == ""
                                    {
                                        self.list_state.select_previous();
                                    }
                                    self.input = true;
                                } else if csi.final_byte == b'B' {
                                    self.list_state.select_next();
                                    if PAGE_MENU_ITEMS
                                        .get(self.list_state.selected().unwrap_or_default())
                                        .cloned()
                                        .unwrap_or_default()
                                        == ""
                                    {
                                        self.list_state.select_next();
                                    }
                                    self.input = true;
                                } else {
                                    trace!("CSI: {:?}", csi);
                                }
                            }
                            event => {
                                trace!("Event: {:?}", event);
                            }
                        },
                    );
                }
                continue;
            }
            if !self.xon {
                return;
            }
            let next = self.pending.pending.borrow_mut().front().map(|&byte| byte);
            if let Some(byte) = next {
                match self.tx.try_send(byte) {
                    Ok(_) => _ = self.pending.pending.borrow_mut().pop_front(),
                    Err(mpsc::TrySendError::Full(_)) => (),
                    Err(mpsc::TrySendError::Disconnected(_)) => (),
                }
                return;
            } else {
                if !self.input {
                    return;
                }
                self.input = false;

                // Move cursor to top-left corner and set double width line for
                // our title (we do this before and after because Ratatui
                // doesn't _really_ support it)
                self.pending.pending.borrow_mut().extend(b"\x1b[0;0H");
                self.pending.pending.borrow_mut().extend(b"\x1b#6");

                _ = self.screen.draw(|f| {
                    let layout = ratatui::layout::Layout::vertical(vec![
                        Constraint::Length(1),
                        Constraint::Fill(1),
                    ]);
                    let areas = layout.split(f.area());
                    f.render_widget(
                        Line::from(vec![Span::from("    Blaze")]).reversed(),
                        areas[0],
                    );

                    let block = Block::bordered()
                        .border_set(VT420_BORDER_SET)
                        .border_style(Style::default())
                        .padding(Padding::symmetric(1, 0));

                    if self.page == 0 {
                        let paragraph = create_demo_text().wrap(Wrap { trim: true }).block(block);
                        f.render_widget(paragraph, areas[1]);
                    } else if self.page == 1 {
                        let list = List::new(PAGE_MENU_ITEMS)
                            .block(
                                block
                                    .title("Display tests")
                                    .title_alignment(HorizontalAlignment::Center),
                            )
                            .style(Style::default())
                            .highlight_style(Style::new().reversed())
                            .highlight_symbol(">>")
                            .repeat_highlight_symbol(true)
                            .direction(ListDirection::TopToBottom);

                        f.render_stateful_widget(list, areas[1], &mut self.list_state);
                    }
                });

                self.pending.pending.borrow_mut().extend(b"\x1b[\"v");

                // Move cursor to top-left corner and set double width line for
                // our title
                self.pending.pending.borrow_mut().extend(b"\x1b[0;0H");
                self.pending.pending.borrow_mut().extend(b"\x1b#6");

                break;
            }
        }
    }
}

fn blank_line<'a>() -> Line<'a> {
    Line::from(vec![])
}

fn line<'a>(spans: &[Span<'a>]) -> Line<'a> {
    Line::from(spans.to_vec())
}

fn span<'a>(text: &'a str) -> Span<'a> {
    Span::from(text)
}

fn bold<'a>(text: &'a str) -> Span<'a> {
    Span::styled(text, Style::default().bold())
}

fn underlined<'a>(text: &'a str) -> Span<'a> {
    Span::styled(text, Style::default().underlined())
}

fn reversed<'a>(text: &'a str) -> Span<'a> {
    Span::styled(text, Style::default().reversed())
}

fn create_demo_text<'a>() -> Paragraph<'a> {
    let mut lines = vec![];
    lines.push(line(&[
        bold("Blaze"),
        span(" is an emulator for the VT420 terminal. "),
        span("This text is displayed by default if you don't configure a connection when starting the emulator. "),
    ]));
    lines.push(blank_line());
    lines.push(line(&[underlined("Tips:")]));
    lines.push(line(&[
        span(" * Press "),
        reversed("F3"),
        span(" to configure the terminal"),
    ]));
    lines.push(line(&[
        span(" * The terminal supports 80/132 columns and 24/36/48 rows (configured under "),
        underlined("Display"),
        span(")."),
    ]));
    lines.push(line(&[
        span(" * For best results, set the "),
        underlined("Page size"),
        span(" to "),
        underlined("1x144"),
        span(" (for 1 session) or "),
        underlined("1x72"),
        span(" (for 2 sessions)."),
    ]));
    lines.push(line(&[
        span(" * To configure multiple sessions, select "),
        underlined("Global > S1=Comm1, S2=Comm2"),
    ]));
    lines.push(line(&[
        span(" * Switch between multiple sessions with "),
        reversed("F4"),
        span(" or split the screen with "),
        reversed("Ctrl+F4"),
        span("."),
    ]));
    lines.push(blank_line());
    lines.push(line(&[
        bold("Blaze"),
        span(" is open-source software written by Matt Mastracci and licensed under the AGPL-3.0 license."),
    ]));
    lines.push(blank_line());
    lines.push(line(&[
        span("Source code is available at "),
        underlined("https://github.com/mmastrac/blaze-vt"),
    ]));
    lines.push(blank_line());
    lines.push(blank_line());
    lines.push(line(&[reversed("[ Press the right arrow key --> ]")]).centered());
    Paragraph::new(lines)
}
