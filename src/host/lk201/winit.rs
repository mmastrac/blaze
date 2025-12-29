use winit::keyboard::{Key, NamedKey};
use winit_input_helper::WinitInputHelper;

use lk201::{LK201Sender, SpecialKey};

pub fn update_keyboard(input: &WinitInputHelper, sender: &LK201Sender) {
    let send = |key| {
        if input.held_control() {
            if input.held_shift() {
                sender.send_shift_ctrl_special_key(key);
            } else {
                sender.send_ctrl_special_key(key);
            }
        } else if input.held_shift() {
            sender.send_shift_special_key(key);
        } else {
            sender.send_special_key(key);
        }
    };

    let send_char = |c| {
        if input.held_control() {
            sender.send_ctrl_char(c as char);
        } else {
            sender.send_char(c as char);
        }
    };

    for key in input.text() {
        match key {
            Key::Named(NamedKey::F1) => send(SpecialKey::F1),
            Key::Named(NamedKey::F2) => send(SpecialKey::F2),
            Key::Named(NamedKey::F3) => send(SpecialKey::F3),
            Key::Named(NamedKey::F4) => send(SpecialKey::F4),
            Key::Named(NamedKey::F5) => send(SpecialKey::F5),
            Key::Named(NamedKey::F6) => send(SpecialKey::F6),
            Key::Named(NamedKey::F7) => send(SpecialKey::F7),
            Key::Named(NamedKey::F8) => send(SpecialKey::F8),
            Key::Named(NamedKey::F9) => send(SpecialKey::F9),
            Key::Named(NamedKey::F10) => send(SpecialKey::F10),
            Key::Named(NamedKey::F11) => send(SpecialKey::F11),
            Key::Named(NamedKey::F12) => send(SpecialKey::F12),
            Key::Named(NamedKey::F13) => send(SpecialKey::F13),
            Key::Named(NamedKey::F14) => send(SpecialKey::F14),
            Key::Named(NamedKey::F15) => send(SpecialKey::Help),
            Key::Named(NamedKey::F16) => send(SpecialKey::Menu),
            Key::Named(NamedKey::F17) => send(SpecialKey::F17),
            Key::Named(NamedKey::F18) => send(SpecialKey::F18),
            Key::Named(NamedKey::F19) => send(SpecialKey::F19),
            Key::Named(NamedKey::F20) => send(SpecialKey::F20),
            Key::Named(NamedKey::ArrowUp) => send(SpecialKey::Up),
            Key::Named(NamedKey::ArrowDown) => send(SpecialKey::Down),
            Key::Named(NamedKey::ArrowLeft) => send(SpecialKey::Left),
            Key::Named(NamedKey::ArrowRight) => send(SpecialKey::Right),
            Key::Named(NamedKey::Enter) => send(SpecialKey::Return),
            Key::Named(NamedKey::Backspace) => send(SpecialKey::Delete),
            Key::Named(NamedKey::Tab) => send(SpecialKey::Tab),
            Key::Named(NamedKey::Home) => send(SpecialKey::Find),
            Key::Named(NamedKey::End) => send(SpecialKey::Select),
            Key::Named(NamedKey::Insert) => send(SpecialKey::InsertHere),
            Key::Named(NamedKey::Delete) => send(SpecialKey::Remove),
            Key::Named(NamedKey::PageUp) => send(SpecialKey::PrevScreen),
            Key::Named(NamedKey::PageDown) => send(SpecialKey::NextScreen),
            Key::Named(NamedKey::NumLock) => send(SpecialKey::KpPf1),

            Key::Named(NamedKey::Escape) => sender.send_escape(),
            Key::Named(NamedKey::Space) => send_char(' '),
            // Key::Named(NamedKey::NumpadDivide) => send(SpecialKey::KpPf2),
            // Key::Named(NamedKey::NumpadMultiply) => send(SpecialKey::KpPf3),
            // Key::Named(NamedKey::NumpadSubtract) => send(SpecialKey::KpPf4),
            // Key::Named(NamedKey::Numpad0) => send(SpecialKey::Kp0),
            // Key::Named(NamedKey::Numpad1) => send(SpecialKey::Kp1),
            // Key::Named(NamedKey::Numpad2) => send(SpecialKey::Kp2),
            // Key::Named(NamedKey::Numpad3) => send(SpecialKey::Kp3),
            // Key::Named(NamedKey::Numpad4) => send(SpecialKey::Kp4),
            // Key::Named(NamedKey::Numpad5) => send(SpecialKey::Kp5),
            // Key::Named(NamedKey::Numpad6) => send(SpecialKey::Kp6),
            // Key::Named(NamedKey::Numpad7) => send(SpecialKey::Kp7),
            // Key::Named(NamedKey::Numpad8) => send(SpecialKey::Kp8),
            // Key::Named(NamedKey::Numpad9) => send(SpecialKey::Kp9),
            // Key::Named(NamedKey::NumpadAdd) => send(SpecialKey::KpHyphen),
            // Key::Named(NamedKey::NumpadDecimal) => send(SpecialKey::KpPeriod),
            // Key::Named(NamedKey::NumpadEnter) => send(SpecialKey::KpEnter),
            Key::Character(c) => {
                if let Some(c) = c.chars().next() {
                    send_char(c)
                }
            }
            _ => {}
        }
    }
}
