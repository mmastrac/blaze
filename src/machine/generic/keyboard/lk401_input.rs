use std::collections::VecDeque;
use std::sync::mpsc;

use tracing::debug;

use lk201::{Key, LK201Sender, SpecialKey};

use crate::machine::generic::keyboard::{KeyCap, KeyboardInput};

pub struct Ps2Keyboard {
    rx: VecDeque<u8>,
    leds: u8,
    expect_arg: Option<u8>,
    pub pc_keyboard: bool,
    pub present: bool,
    /// Scanning is off after 0xF5 (disable) until 0xF4 (enable) or a reset.
    enabled: bool,
    scan_set: u8,
    send: mpsc::Sender<u8>,
    recv: mpsc::Receiver<u8>,
}

impl Default for Ps2Keyboard {
    fn default() -> Self {
        let (send, recv) = mpsc::channel();
        Self {
            rx: VecDeque::new(),
            leds: 0,
            expect_arg: None,
            pc_keyboard: false,
            present: true,
            enabled: true,
            scan_set: 2,
            send,
            recv,
        }
    }
}

impl Ps2Keyboard {
    pub fn input(&self) -> Lk401Input {
        Lk401Input::new(Link::Ps2 {
            send: self.send.clone(),
            lk_escape: !self.pc_keyboard,
        })
    }

    /// Move key bytes from the senders into the receive queue.
    pub fn tick(&mut self) {
        while let Ok(byte) = self.recv.try_recv() {
            if self.enabled {
                self.rx.push_back(byte);
            }
        }
    }

    pub fn command(&mut self, byte: u8) {
        if !self.present {
            return;
        }
        if let Some(cmd) = self.expect_arg.take() {
            debug!("PS/2 command {cmd:02X} argument {byte:02X}");
            match cmd {
                0xED => {
                    self.leds = byte;
                    debug!("PS/2 LEDs = {byte:02X}");
                }
                // Scan code set, 0 for query.
                0xF0 if byte == 0 => {
                    self.rx.extend([0xFA, self.scan_set]);
                    return;
                }
                0xF0 => self.scan_set = byte,
                _ => {}
            }
            self.rx.push_back(0xFA);
            return;
        }
        debug!("PS/2 command {byte:02X}");
        match byte {
            // Reset: ACK, then self-test passed.
            0xFF => {
                self.enabled = true;
                self.scan_set = 2;
                self.rx.extend([0xFA, 0xAA]);
            }
            0xF4 => {
                self.enabled = true;
                self.rx.push_back(0xFA);
            }
            0xF5 => {
                self.enabled = false;
                self.rx.push_back(0xFA);
            }
            // Identify: ACK, then an MF2 keyboard ID.
            0xF2 => self.rx.extend([0xFA, 0xAB, 0x83]),
            0xEE => self.rx.push_back(0xEE),
            0xAF if self.pc_keyboard => self.rx.push_back(0xFE),
            0xED | 0xF0 | 0xF3 => {
                self.expect_arg = Some(byte);
                self.rx.push_back(0xFA);
            }
            _ => self.rx.push_back(0xFA),
        }
    }

    pub fn leds(&self) -> u8 {
        self.leds
    }

    pub fn send(&mut self, bytes: &[u8]) {
        self.rx.extend(bytes);
    }

    pub fn queued(&self) -> usize {
        self.rx.len()
    }

    pub fn has_data(&self) -> bool {
        !self.rx.is_empty()
    }

    pub fn pop(&mut self) -> Option<u8> {
        self.rx.pop_front()
    }
}

/// Sent when the last held up/down key is released (LK201 mode).
const ALL_UP: u8 = 0xB3;
/// Break prefix in scan code set 3.
const BREAK: u8 = 0xF0;

enum Link {
    Lk201(LK201Sender),
    Ps2 {
        send: mpsc::Sender<u8>,
        lk_escape: bool,
    },
}

/// An LK401 keyboard: host key presses sent as LK201 key codes (VT420) or as
/// PS/2 scan code set 3 (VT5xx). DEC's LK keyboards have no Escape key, so an
/// LK keyboard sends Ctrl-3 instead.
pub struct Lk401Input {
    link: Link,
    held: Vec<u8>,
}

impl Lk401Input {
    pub fn lk201(sender: LK201Sender) -> Self {
        Self::new(Link::Lk201(sender))
    }

    fn new(link: Link) -> Self {
        Self { link, held: vec![] }
    }

    fn send(&self, code: u8) {
        match &self.link {
            Link::Lk201(sender) => sender.send_raw(code),
            Link::Ps2 { send, .. } => _ = send.send(code),
        }
    }

    fn lk201_down(&mut self, code: u8) {
        if is_up_down(code) {
            if self.held.contains(&code) {
                return;
            }
            self.held.push(code);
        }
        self.send(code);
    }

    fn lk201_up(&mut self, code: u8) {
        if !is_up_down(code) {
            return;
        }
        let Some(i) = self.held.iter().position(|&c| c == code) else {
            return;
        };
        self.held.remove(i);
        self.send(if self.held.is_empty() { ALL_UP } else { code });
    }

    fn ps2_down(&mut self, code: u8) {
        if self.held.contains(&code) {
            return;
        }
        self.held.push(code);
        self.send(code);
    }

    fn ps2_up(&mut self, code: u8) {
        let Some(i) = self.held.iter().position(|&c| c == code) else {
            return;
        };
        self.held.remove(i);
        self.send(BREAK);
        self.send(code);
    }
}

impl KeyboardInput for Lk401Input {
    fn key_down(&mut self, key: KeyCap) {
        match self.link {
            Link::Lk201(_) => {
                if key == KeyCap::Escape {
                    self.lk201_down(SpecialKey::Ctrl as u8);
                    self.lk201_down(char_code('3'));
                } else if let Some(code) = lk201_code(key) {
                    self.lk201_down(code);
                }
            }
            Link::Ps2 { lk_escape, .. } => {
                if lk_escape && key == KeyCap::Escape {
                    self.key_down(KeyCap::LeftCtrl);
                    self.key_down(KeyCap::Digit3);
                } else if let Some(code) = set3_code(key) {
                    self.ps2_down(code);
                }
            }
        }
    }

    fn key_up(&mut self, key: KeyCap) {
        match self.link {
            Link::Lk201(_) => {
                if key == KeyCap::Escape {
                    self.lk201_up(SpecialKey::Ctrl as u8);
                } else if let Some(code) = lk201_code(key) {
                    self.lk201_up(code);
                }
            }
            Link::Ps2 { lk_escape, .. } => {
                if lk_escape && key == KeyCap::Escape {
                    self.key_up(KeyCap::Digit3);
                    self.key_up(KeyCap::LeftCtrl);
                } else if let Some(code) = set3_code(key) {
                    self.ps2_up(code);
                }
            }
        }
    }
}

fn is_up_down(code: u8) -> bool {
    use SpecialKey::*;
    [Shift, RShift, Ctrl, Lock, Meta, F1, F2, F3, F4, F5]
        .iter()
        .any(|&k| k as u8 == code)
}

fn char_code(c: char) -> u8 {
    Key::char_to_keycode(c).map(|(code, _)| code).unwrap()
}

fn lk201_code(key: KeyCap) -> Option<u8> {
    use KeyCap::*;
    let c = match key {
        Grave => '`',
        Digit1 => '1',
        Digit2 => '2',
        Digit3 => '3',
        Digit4 => '4',
        Digit5 => '5',
        Digit6 => '6',
        Digit7 => '7',
        Digit8 => '8',
        Digit9 => '9',
        Digit0 => '0',
        Minus => '-',
        Equal => '=',
        Q => 'q',
        W => 'w',
        E => 'e',
        R => 'r',
        T => 't',
        Y => 'y',
        U => 'u',
        I => 'i',
        O => 'o',
        P => 'p',
        LeftBracket => '[',
        RightBracket => ']',
        Backslash => '\\',
        A => 'a',
        S => 's',
        D => 'd',
        F => 'f',
        G => 'g',
        H => 'h',
        J => 'j',
        K => 'k',
        L => 'l',
        Semicolon => ';',
        Quote => '\'',
        LessGreater => '<',
        Z => 'z',
        X => 'x',
        C => 'c',
        V => 'v',
        B => 'b',
        N => 'n',
        M => 'm',
        Comma => ',',
        Period => '.',
        Slash => '/',
        Space => ' ',
        _ => return special_key(key).map(|k| k as u8),
    };
    Some(char_code(c))
}

fn special_key(key: KeyCap) -> Option<SpecialKey> {
    use KeyCap::*;
    Some(match key {
        Backspace => SpecialKey::Delete,
        Tab => SpecialKey::Tab,
        CapsLock => SpecialKey::Lock,
        Enter => SpecialKey::Return,
        LeftShift => SpecialKey::Shift,
        RightShift => SpecialKey::RShift,
        LeftCtrl | RightCtrl => SpecialKey::Ctrl,
        Compose => SpecialKey::Meta,
        F1 => SpecialKey::F1,
        F2 => SpecialKey::F2,
        F3 => SpecialKey::F3,
        F4 => SpecialKey::F4,
        F5 => SpecialKey::F5,
        F6 => SpecialKey::F6,
        F7 => SpecialKey::F7,
        F8 => SpecialKey::F8,
        F9 => SpecialKey::F9,
        F10 => SpecialKey::F10,
        F11 => SpecialKey::F11,
        F12 => SpecialKey::F12,
        F13 => SpecialKey::F13,
        F14 => SpecialKey::F14,
        Help => SpecialKey::Help,
        Do => SpecialKey::Menu,
        F17 => SpecialKey::F17,
        F18 => SpecialKey::F18,
        F19 => SpecialKey::F19,
        F20 => SpecialKey::F20,
        Find | Home => SpecialKey::Find,
        InsertHere | Insert => SpecialKey::InsertHere,
        Remove | Delete => SpecialKey::Remove,
        Select | End => SpecialKey::Select,
        PrevScreen | PageUp => SpecialKey::PrevScreen,
        NextScreen | PageDown => SpecialKey::NextScreen,
        Up => SpecialKey::Up,
        Down => SpecialKey::Down,
        Left => SpecialKey::Left,
        Right => SpecialKey::Right,
        KpPf1 | NumLock => SpecialKey::KpPf1,
        KpPf2 | KpDivide => SpecialKey::KpPf2,
        KpPf3 | KpMultiply => SpecialKey::KpPf3,
        KpPf4 => SpecialKey::KpPf4,
        KpMinus => SpecialKey::KpHyphen,
        KpComma | KpPlus => SpecialKey::KpComma,
        KpEnter => SpecialKey::KpEnter,
        KpPeriod => SpecialKey::KpPeriod,
        Kp0 => SpecialKey::Kp0,
        Kp1 => SpecialKey::Kp1,
        Kp2 => SpecialKey::Kp2,
        Kp3 => SpecialKey::Kp3,
        Kp4 => SpecialKey::Kp4,
        Kp5 => SpecialKey::Kp5,
        Kp6 => SpecialKey::Kp6,
        Kp7 => SpecialKey::Kp7,
        Kp8 => SpecialKey::Kp8,
        Kp9 => SpecialKey::Kp9,
        _ => return None,
    })
}

/// Scan code set 3 make code for a key.
fn set3_code(key: KeyCap) -> Option<u8> {
    use KeyCap::*;
    Some(match key {
        Grave => 0x0E,
        Digit1 => 0x16,
        Digit2 => 0x1E,
        Digit3 => 0x26,
        Digit4 => 0x25,
        Digit5 => 0x2E,
        Digit6 => 0x36,
        Digit7 => 0x3D,
        Digit8 => 0x3E,
        Digit9 => 0x46,
        Digit0 => 0x45,
        Minus => 0x4E,
        Equal => 0x55,
        Backspace => 0x66,
        Tab => 0x0D,
        Q => 0x15,
        W => 0x1D,
        E => 0x24,
        R => 0x2D,
        T => 0x2C,
        Y => 0x35,
        U => 0x3C,
        I => 0x43,
        O => 0x44,
        P => 0x4D,
        LeftBracket => 0x54,
        RightBracket => 0x5B,
        Backslash => 0x5C,
        CapsLock => 0x14,
        A => 0x1C,
        S => 0x1B,
        D => 0x23,
        F => 0x2B,
        G => 0x34,
        H => 0x33,
        J => 0x3B,
        K => 0x42,
        L => 0x4B,
        Semicolon => 0x4C,
        Quote => 0x52,
        Enter => 0x5A,
        LeftShift => 0x12,
        LessGreater => 0x13,
        Z => 0x1A,
        X => 0x22,
        C => 0x21,
        V => 0x2A,
        B => 0x32,
        N => 0x31,
        M => 0x3A,
        Comma => 0x41,
        Period => 0x49,
        Slash => 0x4A,
        RightShift => 0x59,
        LeftCtrl => 0x11,
        LeftMeta => 0x8B,
        LeftAlt => 0x19,
        Space => 0x29,
        RightAlt => 0x39,
        RightMeta => 0x8C,
        Menu => 0x8D,
        RightCtrl => 0x58,
        Escape => 0x08,
        F1 => 0x07,
        F2 => 0x0F,
        F3 => 0x17,
        F4 => 0x1F,
        F5 => 0x27,
        F6 => 0x2F,
        F7 => 0x37,
        F8 => 0x3F,
        F9 => 0x47,
        F10 => 0x4F,
        F11 => 0x56,
        F12 => 0x5E,
        PrintScreen => 0x57,
        ScrollLock => 0x5F,
        Pause => 0x62,
        Insert => 0x67,
        Home => 0x6E,
        PageUp => 0x6F,
        Delete => 0x64,
        End => 0x65,
        PageDown => 0x6D,
        Up => 0x63,
        Down => 0x60,
        Left => 0x61,
        Right => 0x6A,
        NumLock => 0x76,
        KpDivide => 0x77,
        KpMultiply => 0x7E,
        KpMinus => 0x84,
        KpPlus => 0x7C,
        KpEnter => 0x79,
        KpPeriod => 0x71,
        Kp0 => 0x70,
        Kp1 => 0x69,
        Kp2 => 0x72,
        Kp3 => 0x7A,
        Kp4 => 0x6B,
        Kp5 => 0x73,
        Kp6 => 0x74,
        Kp7 => 0x6C,
        Kp8 => 0x75,
        Kp9 => 0x7D,
        Compose | F13 | F14 | Help | Do | F17 | F18 | F19 | F20 | Find | InsertHere | Remove
        | Select | PrevScreen | NextScreen | KpComma | KpPf1 | KpPf2 | KpPf3 | KpPf4 => {
            return None;
        }
    })
}
