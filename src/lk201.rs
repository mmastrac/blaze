use std::{collections::VecDeque, sync::mpsc};

use tracing::trace;

pub struct LK201 {
    recv: mpsc::Receiver<u8>,
    send: mpsc::Sender<u8>,
    kbd_queue: VecDeque<u8>,
}

impl LK201 {
    pub fn new(send: mpsc::Sender<u8>, recv: mpsc::Receiver<u8>) -> Self {
        Self {
            send,
            recv,
            kbd_queue: VecDeque::new(),
        }
    }

    pub fn tick(&mut self) {
        if let Ok(value) = self.recv.try_recv() {
            // trace!("KBD: {:02X}", value);
            self.kbd_queue.push_back(value);

            let value = self.kbd_queue.front().unwrap();
            match value {
                // Ping?
                0x55 => {
                    trace!("KBD: Ping");
                    self.kbd_queue.pop_front();
                    _ = self.send.send(1);
                    _ = self.send.send(0);
                    _ = self.send.send(0);
                    _ = self.send.send(0);
                }
                0x11 | 0x13 => {
                    if self.kbd_queue.len() >= 2 {
                        trace!("KBD: LED state");
                        self.kbd_queue.pop_front();
                        self.kbd_queue.pop_front();
                    }
                }
                0x99 => {
                    trace!("KBD: disable click");
                    self.kbd_queue.pop_front();
                    _ = self.send.send(0xba);
                }
                0xAB => {
                    trace!("KBD: Request ID");
                    self.kbd_queue.pop_front();
                    _ = self.send.send(1);
                    _ = self.send.send(0);
                }
                0xE1 => {
                    trace!("KBD: Disable repeat");
                    self.kbd_queue.pop_front();
                    _ = self.send.send(0xba);
                }
                0xE3 => {
                    trace!("KBD: Enable repeat");
                    self.kbd_queue.pop_front();
                    _ = self.send.send(0xba);
                }
                0xFD => {
                    trace!("KBD: Power-up");
                    self.kbd_queue.pop_front();
                    _ = self.send.send(1);
                    _ = self.send.send(0);
                    _ = self.send.send(0);
                    _ = self.send.send(0);
                }
                0x80..=0x8f => {
                    trace!(
                        "KBD: Mode change {:02X} (cmd = {:02X}, division = {})",
                        value,
                        value & 0b1000_0111,
                        (value & 0b0111_1000) >> 3
                    );
                    _ = self.send.send(0xba);
                }
                _ => {
                    trace!("KBD (unknown): {:02X}", value);
                    self.kbd_queue.clear();
                    _ = self.send.send(0xB6);
                }
            }
        }
    }
}
