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
        (KeyCode::ArrowUp, SpecialKey::Up),
        (KeyCode::ArrowDown, SpecialKey::Down),
        (KeyCode::ArrowLeft, SpecialKey::Left),
        (KeyCode::ArrowRight, SpecialKey::Right),
        (KeyCode::Enter, SpecialKey::Return),
        (KeyCode::Backspace, SpecialKey::Delete),
    ] {
        if input.key_pressed(key) {
            if input.held_control() {
                if input.held_shift() {
                    sender.send_shift_ctrl_special_key(mapping);
                } else {
                    sender.send_ctrl_special_key(mapping);
                }
                sender.send_ctrl_special_key(mapping);
            } else if input.held_shift() {
                sender.send_shift_special_key(mapping);
            } else {
                sender.send_special_key(mapping);
            }
        }
    }

    br#"""abcdefghijklmnopqrstuvwxyz0123456789.,-_=+:;'"[]{}\|"""#
        .iter()
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
