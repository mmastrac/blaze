use std::{
    cell::RefCell,
    fmt, io,
    rc::Rc,
    task::{Context, Poll},
};

use ratatui::{
    layout::{Constraint, HorizontalAlignment, Size},
    style::{Style, Stylize},
    symbols::border,
    text::{Line, Span, Text},
    widgets::{Block, List, ListDirection, ListState, Padding, Paragraph, Wrap},
};
use ssu::session::{
    SessionPartsUnsend, SessionRecvEndpoint, SessionSendEndpoint, SessionUnsend,
    loopback::{WakeableQueueRecv, wakeable_queue},
};
use tracing::trace;

use super::ratatui_backend::DecBackend;
use crate::host::demo::ratatui_backend::{VT_DOUBLE_HEIGHT_BOTTOM_LINE, VT_DOUBLE_HEIGHT_TOP_LINE};

const VT420_BORDER_SET: border::Set = border::Set {
    top_left: "\u{00f8}",
    top_right: "\u{00f8}",
    bottom_left: "\u{00ed}",
    bottom_right: "\u{00ea}",
    vertical_left: "\u{00f8}",
    vertical_right: "\u{00f8}",
    horizontal_top: " ",
    horizontal_bottom: "\u{00f1}",
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

pub struct DemoCommSend {
    input_queue: vt_push_parser::VTPushParser,
    pending: DecBackend,
    input: bool,
    page: u8,

    screen: ratatui::Terminal<DecBackend>,
    list_state: ListState,
}

impl fmt::Debug for DemoCommSend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DemoCommSend")
    }
}

pub struct DemoCommRecv {
    queue: WakeableQueueRecv,
}

impl fmt::Debug for DemoCommRecv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DemoCommRecv")
    }
}

pub struct DemoComm;

impl SessionUnsend for DemoComm {
    fn boot(self) -> io::Result<SessionPartsUnsend> {
        let (send, recv) = wakeable_queue();
        let mut pending = DecBackend::new(send);
        pending.size = Rc::new(RefCell::new(Size::new(80, 24)));
        let screen = ratatui::Terminal::new(pending.clone()).unwrap();
        let demo_comm_send = DemoCommSend {
            input_queue: vt_push_parser::VTPushParser::new(),
            screen,
            pending,
            input: false,
            page: 0,
            list_state: ListState::default(),
        };
        let demo_comm_recv = DemoCommRecv { queue: recv };
        Ok(SessionPartsUnsend::new(demo_comm_send, demo_comm_recv))
    }
}

impl SessionSendEndpoint for DemoCommSend {
    fn poll_send(&mut self, _ctx: &mut Context<'_>, byte: u8) -> Poll<io::Result<()>> {
        if byte == 0x0c {
            // ctrl+L - clear screen
            let screen = ratatui::Terminal::new(self.pending.clone()).unwrap();
            self.screen = screen;
            self.input = true;
        } else if byte == 0x0d {
            self.input = true;
            if self.page == 1 {
                match self.list_state.selected() {
                    Some(0) => {
                        self.pending.send_bytes(b"\x1b[80$|");
                    }
                    Some(1) => {
                        self.pending.send_bytes(b"\x1b[132$|");
                    }
                    Some(3) => {
                        self.pending.send_bytes(b"\x1b[24*|");
                    }
                    Some(4) => {
                        self.pending.send_bytes(b"\x1b[36*|");
                    }
                    Some(5) => {
                        self.pending.send_bytes(b"\x1b[48*|");
                    }
                    Some(7) => {
                        self.pending.send_bytes(b"\x1b[24t");
                    }
                    Some(8) => {
                        self.pending.send_bytes(b"\x1b[36t");
                    }
                    Some(9) => {
                        self.pending.send_bytes(b"\x1b[48t");
                    }
                    Some(10) => {
                        self.pending.send_bytes(b"\x1b[72t");
                    }
                    _ => (),
                }
            }
        } else {
            self.input_queue
                .feed_with(
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

                                    trace!("{width}x{height}@{left},{top},{page}");

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

        if self.input {
            self.input = false;

            self.pending.send_bytes(b"\x1b[0;0H");
            self.pending.send_bytes(b"\x1b)0");

            _ = self.screen.draw(|f| {
                let layout = ratatui::layout::Layout::vertical(vec![
                    Constraint::Length(2),
                    Constraint::Fill(1),
                ]);
                let areas = layout.split(f.area());
                f.render_widget(
                    Text::from(vec![
                        Line::from("    Blaze").fg(VT_DOUBLE_HEIGHT_TOP_LINE),
                        Line::from("    Blaze").fg(VT_DOUBLE_HEIGHT_BOTTOM_LINE),
                    ])
                    .reversed(),
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

            self.pending.send_bytes(b"\x1b[\"v");

            // Move cursor to top-left corner
            self.pending.send_bytes(b"\x1b[0;0H");
        }

        Poll::Ready(Ok(()))
    }
}

impl SessionRecvEndpoint for DemoCommRecv {
    fn poll_recv(&mut self, ctx: &mut Context<'_>) -> Poll<io::Result<u8>> {
        self.queue.poll_recv(ctx)
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
        underlined("https://github.com/mmastrac/blaze"),
    ]));
    lines.push(blank_line());
    lines.push(blank_line());
    lines.push(line(&[reversed("[ Press the right arrow key --> ]")]).centered());
    Paragraph::new(lines)
}
