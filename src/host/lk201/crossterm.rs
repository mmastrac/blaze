use ratatui::crossterm::event::{Event, KeyCode, KeyModifiers};

use crate::machine::generic::lk201::{LK201Sender, SpecialKey};

#[derive(Default)]
pub struct CrosstermKeyboard {
    compose_special_key: bool,
}

pub enum KeyboardCommand {
    ToggleRun,
    ToggleHexMode,
    DumpVRAM,
    #[cfg(feature = "pc-trace")]
    TogglePCTrace,
    Quit,
}

impl CrosstermKeyboard {
    pub fn update_keyboard(
        &mut self,
        event: &Event,
        sender: &LK201Sender,
    ) -> Option<KeyboardCommand> {
        if let Event::Key(key) = event {
            if self.compose_special_key {
                self.compose_special_key = false;
                if key.modifiers.is_empty() {
                    match key.code {
                        KeyCode::Char('1') => {
                            _ = sender.send_special_key(SpecialKey::F1);
                        }
                        KeyCode::Char('2') => {
                            _ = sender.send_special_key(SpecialKey::F2);
                        }
                        KeyCode::Char('3') => {
                            _ = sender.send_special_key(SpecialKey::F3);
                        }
                        KeyCode::Char('4') => {
                            _ = sender.send_special_key(SpecialKey::F4);
                        }
                        KeyCode::Char('5') => {
                            _ = sender.send_special_key(SpecialKey::F5);
                        }
                        KeyCode::Char('c') => {
                            _ = sender.send_special_key(SpecialKey::Lock);
                        }
                        KeyCode::Char('q') => {
                            return Some(KeyboardCommand::Quit);
                        }
                        KeyCode::Char(' ') => {
                            return Some(KeyboardCommand::ToggleRun);
                        }
                        KeyCode::Char('h') => {
                            return Some(KeyboardCommand::ToggleHexMode);
                        }
                        KeyCode::Char('d') => {
                            return Some(KeyboardCommand::DumpVRAM);
                        }
                        #[cfg(feature = "pc-trace")]
                        KeyCode::Char('p') => {
                            return Some(KeyboardCommand::TogglePCTrace);
                        }
                        _ => {}
                    }
                }
            }
            if key.modifiers == KeyModifiers::CONTROL {
                match key.code {
                    KeyCode::Char('g') => {
                        self.compose_special_key = true;
                    }
                    KeyCode::Char(c) => {
                        _ = sender.send_ctrl_char(c);
                    }
                    KeyCode::F(1) => {
                        _ = sender.send_ctrl_special_key(SpecialKey::F1);
                    }
                    KeyCode::F(2) => {
                        _ = sender.send_ctrl_special_key(SpecialKey::F2);
                    }
                    KeyCode::F(3) => {
                        _ = sender.send_ctrl_special_key(SpecialKey::F3);
                    }
                    KeyCode::F(4) => {
                        _ = sender.send_ctrl_special_key(SpecialKey::F4);
                    }
                    KeyCode::F(5) => {
                        _ = sender.send_ctrl_special_key(SpecialKey::F5);
                    }
                    KeyCode::Up => {
                        _ = sender.send_ctrl_special_key(SpecialKey::Up);
                    }
                    KeyCode::Down => {
                        _ = sender.send_ctrl_special_key(SpecialKey::Down);
                    }
                    KeyCode::Left => {
                        _ = sender.send_ctrl_special_key(SpecialKey::Left);
                    }
                    KeyCode::Right => {
                        _ = sender.send_ctrl_special_key(SpecialKey::Right);
                    }
                    _ => {}
                }
            }
            if key.modifiers == KeyModifiers::SHIFT | KeyModifiers::CONTROL {
                match key.code {
                    KeyCode::Up => {
                        _ = sender.send_shift_ctrl_special_key(SpecialKey::Up);
                    }
                    KeyCode::Down => {
                        _ = sender.send_shift_ctrl_special_key(SpecialKey::Down);
                    }
                    KeyCode::Left => {
                        _ = sender.send_shift_ctrl_special_key(SpecialKey::Left);
                    }
                    KeyCode::Right => {
                        _ = sender.send_shift_ctrl_special_key(SpecialKey::Right);
                    }
                    _ => {}
                }
            }
            if key.modifiers == KeyModifiers::SHIFT {
                match key.code {
                    KeyCode::Char(c) => {
                        _ = sender.send_char(c);
                    }
                    KeyCode::Up => {
                        _ = sender.send_shift_special_key(SpecialKey::Up);
                    }
                    KeyCode::Down => {
                        _ = sender.send_shift_special_key(SpecialKey::Down);
                    }
                    KeyCode::Left => {
                        _ = sender.send_shift_special_key(SpecialKey::Left);
                    }
                    KeyCode::Right => {
                        _ = sender.send_shift_special_key(SpecialKey::Right);
                    }
                    _ => {}
                }
            }
            if key.modifiers.is_empty() {
                match key.code {
                    KeyCode::Char(c) => {
                        _ = sender.send_char(c);
                    }
                    KeyCode::Left => {
                        _ = sender.send_special_key(SpecialKey::Left);
                    }
                    KeyCode::Right => {
                        _ = sender.send_special_key(SpecialKey::Right);
                    }
                    KeyCode::Up => {
                        _ = sender.send_special_key(SpecialKey::Up);
                    }
                    KeyCode::Down => {
                        _ = sender.send_special_key(SpecialKey::Down);
                    }
                    KeyCode::Backspace => {
                        _ = sender.send_special_key(SpecialKey::Delete);
                    }
                    KeyCode::Enter => {
                        _ = sender.send_special_key(SpecialKey::Return);
                    }
                    KeyCode::Esc => {
                        sender.send_escape();
                    }

                    KeyCode::F(1) => {
                        _ = sender.send_special_key(SpecialKey::F1);
                    }
                    KeyCode::F(2) => {
                        _ = sender.send_special_key(SpecialKey::F2);
                    }
                    KeyCode::F(3) => {
                        _ = sender.send_special_key(SpecialKey::F3);
                    }
                    KeyCode::F(4) => {
                        _ = sender.send_special_key(SpecialKey::F4);
                    }
                    KeyCode::F(5) => {
                        _ = sender.send_special_key(SpecialKey::F5);
                    }
                    _ => {}
                }
            }
        }
        None
    }
}
