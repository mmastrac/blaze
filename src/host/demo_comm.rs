use std::{cell::RefCell, collections::VecDeque, io, rc::Rc, sync::mpsc};

use ratatui::{
    backend::{ClearType, WindowSize},
    buffer::Cell,
    layout::{Constraint, HorizontalAlignment, Position, Size},
    style::{Style, Stylize},
    symbols::border,
    text::{Line, Span},
    widgets::{Block, List, ListDirection, ListState, Padding, Paragraph, TitlePosition, Wrap},
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

#[derive(Clone, Default)]
struct Pending {
    pending: Rc<RefCell<VecDeque<u8>>>,
    size: Rc<RefCell<Size>>,
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

struct BackendWrapper(ratatui::backend::TermionBackend<Pending>, Pending);

impl ratatui::backend::Backend for BackendWrapper {
    type Error = io::Error;
    fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        self.0.draw(content)
    }

    fn hide_cursor(&mut self) -> Result<(), Self::Error> {
        self.0.hide_cursor()
    }

    fn show_cursor(&mut self) -> Result<(), Self::Error> {
        self.0.show_cursor()
    }

    fn get_cursor_position(&mut self) -> Result<Position, Self::Error> {
        self.0.get_cursor_position()
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> Result<(), Self::Error> {
        self.0.set_cursor_position(position)
    }

    fn clear(&mut self) -> Result<(), Self::Error> {
        self.0.clear()
    }

    fn clear_region(&mut self, clear_type: ClearType) -> Result<(), Self::Error> {
        self.0.clear_region(clear_type)
    }

    fn size(&self) -> Result<Size, Self::Error> {
        Ok(*self.1.size.borrow())
    }

    fn window_size(&mut self) -> Result<WindowSize, Self::Error> {
        Ok(WindowSize {
            columns_rows: *self.1.size.borrow(),
            pixels: Size::new(10, 16),
        })
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.0.flush()
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

    screen: ratatui::Terminal<BackendWrapper>,
    list_state: ListState,
}

impl DemoComm {
    pub fn new(tx: mpsc::SyncSender<u8>, rx: mpsc::Receiver<u8>) -> Self {
        let mut pending = Pending::default();
        pending.size = Rc::new(RefCell::new(Size::new(80, 24)));
        let screen = ratatui::Terminal::new(BackendWrapper(
            ratatui::backend::TermionBackend::new(pending.clone()),
            pending.clone(),
        ))
        .unwrap();
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
                    let screen = ratatui::Terminal::new(BackendWrapper(
                        ratatui::backend::TermionBackend::new(self.pending.clone()),
                        self.pending.clone(),
                    ))
                    .unwrap();
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

fn create_demo_text<'a>() -> Paragraph<'a> {
    let mut lines = vec![];
    lines.push(Line::from(vec![
        Span::styled("Blaze", Style::default().bold()),
        Span::from(
            " is an emulator for the VT420 terminal. "
        ),
        Span::from(
            "This text is displayed by default if you don't configure a connection when starting the emulator. "
        ),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled(
        "Tips:",
        Style::default().underlined(),
    )]));
    lines.push(Line::from(vec![
        Span::from(" * Press "),
        Span::styled("F3", Style::default().reversed()),
        Span::from(" to configure the terminal"),
    ]));
    lines.push(Line::from(vec![
        Span::from(" * The terminal supports 80/132 columns and 24/36/48 rows (configured under "),
        Span::styled("Display", Style::default().underlined()),
        Span::from("). For best results, set the "),
        Span::styled("Page size", Style::default().underlined()),
        Span::from(" to "),
        Span::styled("1x144", Style::default().underlined()),
        Span::from("."),
    ]));
    lines.push(Line::from(vec![
        Span::from(" * To configure multiple sessions, select "),
        Span::styled("Global > S1=Comm1, S2=Comm2", Style::default().underlined()),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("Blaze", Style::default().bold()),
        Span::from(" is open source software licensed under the AGPL-3.0 license."),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::from("Source code is available at "),
        Span::styled(
            "https://github.com/mmastrac/blaze-vt",
            Style::default().underlined(),
        ),
    ]));

    lines.push(Line::from(""));
    lines.push(Line::from(""));
    lines.push(
        Line::styled(
            "[ Press the right arrow key --> ]",
            Style::default().reversed(),
        )
        .centered(),
    );
    Paragraph::new(lines)
}
