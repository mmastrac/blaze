use std::collections::VecDeque;

use tracing::warn;

use crate::{INTRO, OP_ADDCR, OP_PROBE, OP_SELECT, TERM};

/// The maximum number of bytes to send in a single chunk before
/// trying to poll the other session.
const CHUNK_SIZE: usize = 32;

const RECV_CREDITS_LOW_WATER_MARK: usize = 128;
const RECV_CREDITS_TOP_UP: usize = 1024;

/// SSU server state machine implementation. This speaks SSU to a peer and
/// provides multiple SSU endpoints as sessions.
///
/// The downstream sessions are assumed to accept unlimited data.
pub struct Server {
    sessions: Vec<Session>,

    peer_queue: VecDeque<u8>,

    command_buffer: VecDeque<u8>,

    /// If the peer sends XOFF, we stop sending for a bit
    xoff: bool,

    enabled: bool,

    active_recv_session: Option<u8>,
    active_send_session: Option<u8>,
}

impl Server {
    pub fn new(max_sessions: u8) -> Self {
        let mut peer_queue = VecDeque::new();
        peer_queue.extend(b"!@A");
        peer_queue.push_back(b'A' + max_sessions - 1);
        Self {
            sessions: Vec::new(),
            peer_queue,
            xoff: false,
            enabled: false,
            command_buffer: VecDeque::new(),
            active_recv_session: None,
            active_send_session: None,
        }
    }

    /// Mark the peer as idle, resetting partial commands.
    pub fn idle(&mut self) {
        self.command_buffer.clear();
        self.active_recv_session = None;
        self.active_send_session = None;
        self.xoff = false;
    }

    pub fn accept_peer_byte(&mut self, b: u8) {
        if self.command_buffer.is_empty() {
            // Command mode
            if b == INTRO {
                self.command_buffer.push_back(b);
            } else if let Some(active_session) = self.active_recv_session {
                // Session mode
                if let Some(session) = self.sessions.get_mut(active_session as usize) {
                    session.recv_queue.push_back(b);
                    session.recv_credits = session.recv_credits.saturating_sub(1);
                    if session.recv_credits < RECV_CREDITS_LOW_WATER_MARK {
                        session.recv_credits =
                            session.recv_credits.saturating_add(RECV_CREDITS_TOP_UP);
                        send_add_credits_message(
                            &mut self.peer_queue,
                            session.idx,
                            RECV_CREDITS_TOP_UP,
                        );
                    }
                } else {
                    // Ignore this byte
                }
            }
        } else {
            if b == TERM {
                // Process command
                self.command_buffer.pop_front();
                match self.command_buffer.pop_front().unwrap_or_default() {
                    _ => {
                        warn!("Unknown command: {:?}", self.command_buffer);
                    }
                }
            } else {
                self.command_buffer.push_back(b);
            }
        }
    }

    /// Drain the sessions that have data to send.
    pub fn poll_sessions(&mut self) {
        for session in self.sessions.iter_mut() {
            if !session.send_queue.is_empty() {
                if self.active_send_session != Some(session.idx) {
                    self.peer_queue.push_back(INTRO);
                    self.peer_queue.push_back(OP_SELECT);
                    self.peer_queue.push_back(session.idx + b'A');
                    self.peer_queue.push_back(TERM);
                    self.active_send_session = Some(session.idx);
                }

                if session.send_queue.len() > CHUNK_SIZE {
                    self.peer_queue
                        .extend(session.send_queue.drain(..CHUNK_SIZE));
                } else {
                    self.peer_queue.extend(session.send_queue.drain(..));
                }
            }
        }
    }

    pub fn next_peer_byte(&mut self) -> Option<u8> {
        self.peer_queue.pop_front()
    }
}

pub struct Session {
    idx: u8,
    name: String,
    send_credits: usize,
    recv_credits: usize,
    recv_queue: VecDeque<u8>,
    send_queue: VecDeque<u8>,
}

impl Session {
    /// Should backpressure be applied to this session?
    pub fn backpressure(&self) -> bool {
        self.send_queue.len() > self.send_credits
    }

    pub fn accept_session_byte(&mut self, b: u8) {
        self.recv_queue.push_back(b);
    }

    pub fn next_session_byte(&mut self) -> Option<u8> {
        self.send_queue.pop_front()
    }
}

fn send_probe_message(queue: &mut VecDeque<u8>, protocol_variant: u8, max_sessions: u8) {
    queue.push_back(INTRO);
    queue.push_back(OP_PROBE);
    queue.push_back(b'A' + protocol_variant);
    queue.push_back(b'A' + max_sessions - 1);
    queue.push_back(TERM);
}

fn send_select_message(queue: &mut VecDeque<u8>, session_id: u8) {
    queue.push_back(INTRO);
    queue.push_back(OP_SELECT);
    queue.push_back(b'A' + session_id);
    queue.push_back(TERM);
}

fn send_add_credits_message(queue: &mut VecDeque<u8>, session_id: u8, credits: usize) {
    queue.push_back(INTRO);
    queue.push_back(OP_ADDCR);
    queue.push_back(b'A' + session_id);

    // Credits = { z5, x4, x3, x2, x1, x0, y4, y3, y2, y1, y0, z4, z3, z2, z1, z0 }

    let total = credits as u16;
    let x = ((total >> 10) & 0x1F) as u8;
    let y = ((total >> 5) & 0x1F) as u8;
    let mut z = (total & 0x1F) as u8;

    if (total & 0x8000) != 0 {
        z |= 0x20; // set z5 (bit5 of z byte)
    }

    queue.push_back(x + b'@');
    queue.push_back(y + b'@');
    queue.push_back(z + b'@');
    queue.push_back(TERM);
}
