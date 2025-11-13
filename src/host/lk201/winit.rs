use game_loop::winit::keyboard::{Key, KeyCode};
use winit_input_helper::WinitInputHelper;

use crate::machine::generic::lk201::{LK201Sender, SpecialKey};

pub fn update_keyboard(input: &WinitInputHelper, sender: &LK201Sender) {
    for (key, mapping) in [
        (KeyCode::F1, SpecialKey::F1),
        (KeyCode::F2, SpecialKey::F2),
        (KeyCode::F3, SpecialKey::F3),
        (KeyCode::F4, SpecialKey::F4),
        (KeyCode::F5, SpecialKey::F5),
        (KeyCode::F6, SpecialKey::F6),
        (KeyCode::F7, SpecialKey::F7),
        (KeyCode::F8, SpecialKey::F8),
        (KeyCode::F9, SpecialKey::F9),
        (KeyCode::F10, SpecialKey::F10),
        (KeyCode::F11, SpecialKey::F11),
        (KeyCode::F12, SpecialKey::F12),
        (KeyCode::F13, SpecialKey::F13),
        (KeyCode::F14, SpecialKey::F14),
        (KeyCode::F15, SpecialKey::Help),
        (KeyCode::F16, SpecialKey::Menu),
        (KeyCode::F17, SpecialKey::F17),
        (KeyCode::F18, SpecialKey::F18),
        (KeyCode::F19, SpecialKey::F19),
        (KeyCode::F20, SpecialKey::F20),
        (KeyCode::ArrowUp, SpecialKey::Up),
        (KeyCode::ArrowDown, SpecialKey::Down),
        (KeyCode::ArrowLeft, SpecialKey::Left),
        (KeyCode::ArrowRight, SpecialKey::Right),
        (KeyCode::Enter, SpecialKey::Return),
        (KeyCode::Backspace, SpecialKey::Delete),
        (KeyCode::Tab, SpecialKey::Tab),
        (KeyCode::Home, SpecialKey::Find),
        (KeyCode::End, SpecialKey::Select),
        (KeyCode::Insert, SpecialKey::InsertHere),
        (KeyCode::Delete, SpecialKey::Remove),
        (KeyCode::PageUp, SpecialKey::PrevScreen),
        (KeyCode::PageDown, SpecialKey::NextScreen),
        (KeyCode::NumLock, SpecialKey::KpPf1),
        (KeyCode::NumpadDivide, SpecialKey::KpPf2),
        (KeyCode::NumpadMultiply, SpecialKey::KpPf3),
        (KeyCode::NumpadSubtract, SpecialKey::KpPf4),
        (KeyCode::Numpad0, SpecialKey::Kp0),
        (KeyCode::Numpad1, SpecialKey::Kp1),
        (KeyCode::Numpad2, SpecialKey::Kp2),
        (KeyCode::Numpad3, SpecialKey::Kp3),
        (KeyCode::Numpad4, SpecialKey::Kp4),
        (KeyCode::Numpad5, SpecialKey::Kp5),
        (KeyCode::Numpad6, SpecialKey::Kp6),
        (KeyCode::Numpad7, SpecialKey::Kp7),
        (KeyCode::Numpad8, SpecialKey::Kp8),
        (KeyCode::Numpad9, SpecialKey::Kp9),
        (KeyCode::NumpadAdd, SpecialKey::KpHyphen),
        (KeyCode::NumpadDecimal, SpecialKey::KpPeriod),
        (KeyCode::NumpadEnter, SpecialKey::KpEnter),
    ] {
        if input.key_pressed(key) {
            if input.held_control() {
                if input.held_shift() {
                    sender.send_shift_ctrl_special_key(mapping);
                } else {
                    sender.send_ctrl_special_key(mapping);
                }
            } else if input.held_shift() {
                sender.send_shift_special_key(mapping);
            } else {
                sender.send_special_key(mapping);
            }
            return;
        }
    }

    br#"!"$#%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\]^_`abcdefghijklmnopqrstuvwxyz{|}~"#        .iter()
        .for_each(|&c| {
            let s = &[c];
            let s = str::from_utf8(s).unwrap();
            if input.key_pressed_logical(Key::Character(s)) {
                if input.held_control() {
                    sender.send_ctrl_char(c as char);
                } else {
                    sender.send_char(c as char);
                }
            }
        });

    if input.key_pressed(KeyCode::Space) {
        sender.send_char(' ');
    }

    if input.key_pressed(KeyCode::Escape) {
        sender.send_escape();
    }
}
