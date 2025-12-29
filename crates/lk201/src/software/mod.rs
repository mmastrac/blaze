use std::{collections::VecDeque, sync::mpsc};
use tracing::trace;

use crate::{Key, LK201Command, SpecialKey};

pub struct LK201Sender {
    send: mpsc::Sender<u8>,
}

impl LK201Sender {
    fn new(send: mpsc::Sender<u8>) -> Self {
        Self { send }
    }

    pub fn send_special_key(&self, key: SpecialKey) {
        _ = self.send.send(key as u8);
    }

    pub fn send_char(&self, c: char) {
        if let Some((key, shift)) = Key::char_to_keycode(c) {
            if shift {
                _ = self.send.send(0xae); // shift
            }
            _ = self.send.send(key as u8);
            if shift {
                _ = self.send.send(0xb3); // all up
            }
        }
    }

    pub fn send_ctrl_char(&self, c: char) {
        if let Some((keycode, shift)) = Key::char_to_keycode(c) {
            _ = self.send.send(0xaf); // ctrl
            if shift {
                _ = self.send.send(0xae); // shift
            }
            _ = self.send.send(keycode as u8);
            _ = self.send.send(0xb3); // all up
        }
    }

    pub fn send_ctrl_special_key(&self, key: SpecialKey) {
        _ = self.send.send(0xaf); // ctrl
        _ = self.send.send(key as u8);
        _ = self.send.send(0xb3); // all up
    }

    pub fn send_shift_special_key(&self, key: SpecialKey) {
        _ = self.send.send(0xae); // shift
        _ = self.send.send(key as u8);
        _ = self.send.send(0xb3); // all up
    }

    pub fn send_shift_ctrl_special_key(&self, key: SpecialKey) {
        _ = self.send.send(0xaf); // ctrl
        _ = self.send.send(0xae); // shift
        _ = self.send.send(key as u8);
        _ = self.send.send(0xb3); // all up
    }

    pub fn send_escape(&self) {
        _ = self.send.send(0xaf); // ctrl
        _ = self.send.send(0xcb); // 3
        _ = self.send.send(0xb3); // all up
    }
}

pub struct LK201 {
    recv: mpsc::Receiver<u8>,
    send: mpsc::Sender<u8>,
    kbd_queue: VecDeque<u8>,
    collect_commands: bool,
    collected_bytes: Vec<u8>,
    collected_commands: Vec<LK201Command>,
}

impl LK201 {
    pub fn new(send: mpsc::Sender<u8>, recv: mpsc::Receiver<u8>) -> Self {
        Self {
            send,
            recv,
            kbd_queue: VecDeque::new(),
            collect_commands: false,
            collected_bytes: Vec::new(),
            collected_commands: Vec::new(),
        }
    }

    pub fn start_collecting_commands(&mut self) {
        self.collect_commands = true;
    }

    pub fn stop_collecting_commands(&mut self) -> (Vec<u8>, Vec<LK201Command>) {
        self.collect_commands = false;
        (
            std::mem::take(&mut self.collected_bytes),
            std::mem::take(&mut self.collected_commands),
        )
    }

    pub fn sender(&self) -> LK201Sender {
        LK201Sender::new(self.send.clone())
    }

    pub fn tick(&mut self) {
        // Accumulate incoming bytes
        let mut received = false;
        while let Ok(byte) = self.recv.try_recv() {
            if self.collect_commands {
                self.collected_bytes.push(byte);
            }
            self.kbd_queue.push_back(byte);
            received = true;
        }

        // Try to parse a command from the queue
        if self.kbd_queue.is_empty() || !received {
            return;
        }

        // Attempt to parse command
        let Ok(command) = LK201Command::try_from(&self.kbd_queue) else {
            return;
        };

        if self.collect_commands {
            self.collected_commands.push(command.clone());
        }

        // Successfully parsed a command
        let cmd_len = command.len();

        trace!("KBD: Command {:?}", command);

        // Remove the command bytes from the queue
        for _ in 0..cmd_len {
            self.kbd_queue.pop_front();
        }

        // Send response if the command has one
        if let Some(response) = command.response() {
            trace!(
                "KBD: Sending response {:?} = {:02X?}",
                response,
                response.to_bytes()
            );
            for byte in response.to_bytes() {
                _ = self.send.send(byte);
            }
        }
    }
}
