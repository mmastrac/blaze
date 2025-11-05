//! # LK201 Keyboard Emulator (<https://en.wikipedia.org/wiki/LK201>).
//!
//! The hardware interface is documented in
//! <https://www.netbsd.org/docs/Hardware/Machines/DEC/lk201.html>, and some
//! bootup sequences are documented at <https://vt100.net/keyboard.html>.
#![allow(unused)]

use std::{collections::VecDeque, fmt, sync::mpsc};

use tracing::trace;

/// LED indicators on the LK201 keyboard
///
/// The LED parameter byte is a bitmask: 0x80 | (led_bits)
/// - Bit 7 (0x80): PARAM bit (always set)
/// - Bit 3 (0x08): Hold LED
/// - Bit 2 (0x04): Lock LED  
/// - Bit 1 (0x02): Compose LED
/// - Bit 0 (0x01): Wait LED
///
/// Any combination of LEDs (0x80-0x8F) is valid. Common combinations are named,
/// but Custom(byte) accepts any bitmask value.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Led(u8);

impl Led {
    pub fn new(byte: u8) -> Self {
        Led(byte)
    }

    pub fn is_wait(&self) -> bool {
        self.0 & 0x01 == 0x01
    }

    pub fn is_compose(&self) -> bool {
        self.0 & 0x02 == 0x02
    }

    pub fn is_lock(&self) -> bool {
        self.0 & 0x04 == 0x04
    }

    pub fn is_hold(&self) -> bool {
        self.0 & 0x08 == 0x08
    }

    pub fn is_all(&self) -> bool {
        self.0 & 0x0F == 0x0F
    }
}

impl fmt::Debug for Led {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Led({:02X}=", self.0)?;
        let mut first = true;
        for led in [
            ("Wait", self.is_wait()),
            ("Compose", self.is_compose()),
            ("Lock", self.is_lock()),
            ("Hold", self.is_hold()),
        ] {
            if led.1 {
                if first {
                    first = false;
                } else {
                    write!(f, "+")?;
                }
                write!(f, "{}", led.0)?;
            }
        }
        write!(f, ")")?;
        Ok(())
    }
}

/// Key mode for a keyboard division
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyMode {
    /// Key down generates code once, no repeat
    Down = 0x80,
    /// Key down generates code, then repeats with LK_REPEAT (0xB4)
    AutoDown = 0x82,
    /// Key down/up both generate codes (for modifier keys)
    UpDown = 0x86,
}

/// Keyboard division (1-14, corresponding to groups of keys)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Division(pub u8);

impl Division {
    pub fn new(div: u8) -> Option<Self> {
        if div <= 14 { Some(Division(div)) } else { None }
    }
}

/// Auto-repeat register (0-3)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutoRepeatRegister(pub u8);

impl AutoRepeatRegister {
    pub fn new(reg: u8) -> Option<Self> {
        if reg <= 3 {
            Some(AutoRepeatRegister(reg))
        } else {
            None
        }
    }
}

/// Volume level (0 = max, 7 = min)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Volume(pub u8);

impl Volume {
    pub fn new(level: u8) -> Option<Self> {
        if level <= 7 {
            Some(Volume(level))
        } else {
            None
        }
    }

    pub fn as_param_byte(self) -> u8 {
        0x80 | (self.0 & 0x7)
    }
}

/// Commands sent from the computer to the LK201 keyboard
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LK201Command {
    // LED Control
    /// Enable specified LED(s)
    LedEnable(Led),
    /// Disable specified LED(s)
    LedDisable(Led),

    // Key Click Control
    /// Enable key clicks with specified volume
    KeyClickEnable(Volume),
    /// Enable key clicks for CTRL key
    CtrlKeyClickEnable,
    /// Disable all key clicks
    KeyClickDisable,
    /// Disable CTRL key clicks
    CtrlKeyClickDisable,
    /// Cause keyboard to generate a single click sound
    SoundClick,

    // Bell Control
    /// Enable bell with specified volume
    BellEnable(Volume),
    /// Disable bell
    BellDisable,
    /// Ring the bell
    RingBell,

    // Key Mode Control
    /// Set mode for a keyboard division
    SetMode { mode: KeyMode, division: Division },
    /// Set mode and associate with auto-repeat register
    SetModeWithAutoRepeat {
        mode: KeyMode,
        division: Division,
        register: AutoRepeatRegister,
    },
    /// Convert all LK_AUTODOWN divisions to LK_DOWN
    RepeatToDown,
    /// Enable auto-repeat on LK_AUTODOWN divisions
    EnableRepeat { division: Division },
    /// Disable auto-repeat on all keys
    DisableRepeat { division: Division },
    /// Temporarily disable auto-repeat for currently pressed key
    TempNoRepeat,

    // Auto-repeat Control
    /// Set auto-repeat parameters for a register
    SetAutoRepeat {
        register: AutoRepeatRegister,
        /// Timeout before repeat starts (in 5ms increments, 1-126)
        timeout: u8,
        /// Repeat rate in Hz (12-125, but not 125)
        rate: u8,
    },

    // Power-Up and Self Test
    /// Initiate keyboard power-up sequence
    PowerUp,
    /// Request keyboard ID (2 bytes returned)
    RequestId,
    /// Restore keyboard to default state
    SetDefaults,
    /// Enter factory test mode
    TestMode,
    /// Exit factory test mode (0x80, also used as LK_PARAM bit)
    TestExit,

    // Keyboard Control
    /// Suspend keyboard transmission, turn on LOCK LED
    Inhibit,
    /// Resume normal keyboard operation
    Resume,

    /// Unknown initial byte
    Unknown(u8),
    /// Unknown 2-byte command
    Unknown2(u8, u8),
    /// Unknown 3-byte command (e.g., 0xE9 xx xx - possibly division-specific repeat control)
    Unknown3(u8, u8, u8),
}

impl LK201Command {
    /// Returns the number of bytes this command occupies
    pub fn len(&self) -> usize {
        match self {
            LK201Command::LedEnable(_) => 2,
            LK201Command::LedDisable(_) => 2,
            LK201Command::KeyClickEnable(_) => 2,
            LK201Command::CtrlKeyClickEnable => 1,
            LK201Command::KeyClickDisable => 1,
            LK201Command::CtrlKeyClickDisable => 1,
            LK201Command::SoundClick => 1,
            LK201Command::BellEnable(_) => 2,
            LK201Command::BellDisable => 1,
            LK201Command::RingBell => 1,
            LK201Command::SetMode { .. } => 1,
            LK201Command::SetModeWithAutoRepeat { .. } => 2,
            LK201Command::RepeatToDown => 1,
            LK201Command::EnableRepeat { .. } => 1,
            LK201Command::DisableRepeat { .. } => 1,
            LK201Command::TempNoRepeat => 1,
            LK201Command::SetAutoRepeat { .. } => 3,
            LK201Command::PowerUp => 1,
            LK201Command::RequestId => 1,
            LK201Command::SetDefaults => 1,
            LK201Command::TestMode => 1,
            LK201Command::TestExit => 1,
            LK201Command::Inhibit => 1,
            LK201Command::Resume => 1,
            LK201Command::Unknown(_) => 1,
            LK201Command::Unknown2(_, _) => 2,
            LK201Command::Unknown3(_, _, _) => 3,
        }
    }

    /// Returns the response the keyboard should send for this command, if any.
    ///
    /// Based on the LK201 documentation and Linux kernel driver:
    /// - Power-up commands return multi-byte sequences
    /// - Mode-related commands return ModeChangeAck (0xBA)
    /// - Test/Inhibit commands return specific acks
    /// - Invalid commands return InputError (0xB6)
    /// - Most other commands (LED, bell, click) have no response
    pub fn response(&self) -> Option<LK201Response> {
        Some(match self {
            // Power-up and ID requests return multi-byte responses
            LK201Command::PowerUp => LK201Response::PowerUpSelfTest {
                keyboard_id_firmware: 0x01, // Standard LK201 firmware ID
                keyboard_id_hardware: 0x00, // Hardware ID from jumpers
                error: PowerUpError::NoError,
                keycode: 0,
            },
            LK201Command::RequestId => LK201Response::KeyboardId {
                firmware_id: 0x01, // Firmware version
                hardware_id: 0x01, // 1 = LK201, 2 = LK401, 3 = LK443, 4 = LK421
            },

            // Mode change commands return ModeChangeAck (0xBA)
            // "Upon successful receipt of the command, the LK201 responds with LK_MODECHG_ACK"
            LK201Command::SetMode { .. } => LK201Response::ModeChangeAck,
            LK201Command::SetModeWithAutoRepeat { .. } => LK201Response::ModeChangeAck,
            LK201Command::SetAutoRepeat { .. } => LK201Response::ModeChangeAck,

            // Repeat control commands that change mode behavior
            LK201Command::RepeatToDown => LK201Response::ModeChangeAck,
            LK201Command::TempNoRepeat => LK201Response::ModeChangeAck,
            LK201Command::EnableRepeat { .. } => LK201Response::ModeChangeAck,
            LK201Command::DisableRepeat { .. } => LK201Response::ModeChangeAck,

            // Special control commands with specific acks
            LK201Command::TestMode => LK201Response::TestModeAck,
            LK201Command::Inhibit => LK201Response::KeyboardLockAck,

            // Invalid commands return InputError
            LK201Command::Unknown(_) => LK201Response::InputError,

            // All other commands (LED, bell, click, autorepeat rate, etc.) have no response
            _ => return None,
        })
    }
}

impl TryFrom<&VecDeque<u8>> for LK201Command {
    type Error = ();

    fn try_from(value: &VecDeque<u8>) -> Result<Self, Self::Error> {
        let Some(&byte0) = value.get(0) else {
            return Err(());
        };

        match byte0 {
            // LED Control
            0x13 => {
                let Some(&led_byte) = value.get(1) else {
                    return Err(());
                };
                let led = Led::new(led_byte);
                Ok(LK201Command::LedEnable(led))
            }
            0x11 => {
                let Some(&led_byte) = value.get(1) else {
                    return Err(());
                };
                let led = Led::new(led_byte);
                Ok(LK201Command::LedDisable(led))
            }

            // Key Click Control
            0x1B => {
                let Some(&vol_byte) = value.get(1) else {
                    return Err(());
                };
                let Some(volume) = Volume::new((vol_byte & 0x7) as u8) else {
                    return Ok(LK201Command::Unknown2(0x1B, vol_byte));
                };
                Ok(LK201Command::KeyClickEnable(volume))
            }
            0xBB => Ok(LK201Command::CtrlKeyClickEnable),
            0x99 => Ok(LK201Command::KeyClickDisable),
            0xB9 => Ok(LK201Command::CtrlKeyClickDisable),
            0x9F => Ok(LK201Command::SoundClick),

            // Bell Control
            0x23 => {
                let Some(&vol_byte) = value.get(1) else {
                    return Err(());
                };
                let Some(volume) = Volume::new((vol_byte & 0x7) as u8) else {
                    return Ok(LK201Command::Unknown2(0x23, vol_byte));
                };
                Ok(LK201Command::BellEnable(volume))
            }
            0xA1 => Ok(LK201Command::BellDisable),
            0xA7 => Ok(LK201Command::RingBell),

            // Autorepeat rate commands (0x78-0x7F range: bits 6-3 = 1111, bits 2-1 = register)
            0x78..=0x7F if (byte0 >> 3) & 0xF == 0xF => {
                let Some(&timeout) = value.get(1) else {
                    return Err(());
                };
                let Some(&rate) = value.get(2) else {
                    return Err(());
                };
                let Some(register) = AutoRepeatRegister::new((byte0 >> 1) & 0x3) else {
                    return Ok(LK201Command::Unknown2(0x78, byte0));
                };
                Ok(LK201Command::SetAutoRepeat {
                    register,
                    timeout: timeout & 0x7F,
                    rate: rate & 0x7F,
                })
            }

            // Key Mode Control
            // Check if this is a mode command by examining the structure
            // Note: 0x80 (TestExit) has the same bit pattern as SetMode{division:0, mode:Down}
            // It's only interpreted as TestExit when the keyboard is already in test mode
            b if (b & 0x01) == 0 => {
                // Bit 0 is 0, could be a mode command
                let division_bits = (b >> 3) & 0xF;
                let mode_bits = (b >> 1) & 0x3;
                let has_param = (b & 0x80) == 0;

                // Check if division is valid (0-14)
                if division_bits <= 14 {
                    let Some(division) = Division::new(division_bits) else {
                        return Ok(LK201Command::Unknown(b));
                    };
                    let mode = match mode_bits {
                        0b00 => KeyMode::Down,
                        0b01 => KeyMode::AutoDown,
                        0b11 => KeyMode::UpDown,
                        _ => return Ok(LK201Command::Unknown(b)),
                    };

                    // Check PARAM bit (bit 7)
                    if has_param {
                        // PARAM = 0, parameter follows with autorepeat register
                        let Some(&param_byte) = value.get(1) else {
                            return Err(());
                        };
                        let Some(register) = AutoRepeatRegister::new(param_byte & 0x3) else {
                            return Ok(LK201Command::Unknown2(b, param_byte));
                        };
                        return Ok(LK201Command::SetModeWithAutoRepeat {
                            mode,
                            division,
                            register,
                        });
                    } else {
                        // PARAM = 1, no parameter
                        return Ok(LK201Command::SetMode { mode, division });
                    }
                }

                // Not a valid mode command, fall through
                Ok(LK201Command::Unknown(byte0))
            }

            // Other Commands
            0xD9 => Ok(LK201Command::RepeatToDown),
            0xD1 => Ok(LK201Command::TempNoRepeat),

            0xE1..0xEF => {
                let division_bits = (byte0 >> 3) & 0b111;
                let Some(division) = Division::new(division_bits) else {
                    return Ok(LK201Command::Unknown(byte0));
                };
                if byte0 & 0x02 == 0 {
                    Ok(LK201Command::DisableRepeat { division })
                } else {
                    Ok(LK201Command::EnableRepeat { division })
                }
            }

            // Power-Up and Self Test
            0xFD => Ok(LK201Command::PowerUp),
            0xAB => Ok(LK201Command::RequestId),
            0xD3 => Ok(LK201Command::SetDefaults),
            0xCB => Ok(LK201Command::TestMode),

            // Keyboard Control
            0x8B => Ok(LK201Command::Resume),
            0x89 => Ok(LK201Command::Inhibit),

            // Unknown command
            _ => Ok(LK201Command::Unknown(byte0)),
        }
    }
}

/// Special keycodes that have specific meanings
pub mod keycodes {
    /// Caps Lock key
    pub const KEY_LOCK: u8 = 0xB0;
    /// Shift key
    pub const KEY_SHIFT: u8 = 0xAE;
    /// Control key
    pub const KEY_CTRL: u8 = 0xAF;
    /// Compose key
    pub const KEY_COMP: u8 = 0xB1;
}

/// Responses sent from the LK201 keyboard to the computer
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LK201Response {
    // Power-up and ID
    /// Power-up self test result (4 bytes: ID1, ID2, error, keycode)
    PowerUpSelfTest {
        keyboard_id_firmware: u8,
        keyboard_id_hardware: u8,
        error: PowerUpError,
        keycode: u8,
    },
    /// Keyboard ID response (2 bytes)
    KeyboardId { firmware_id: u8, hardware_id: u8 },

    // Acknowledgments
    /// Mode change acknowledged
    ModeChangeAck,
    /// Keyboard locked acknowledged
    KeyboardLockAck,
    /// Test mode acknowledged
    TestModeAck,

    // Errors
    /// Input error (invalid command received)
    InputError,
    /// Output error (keystrokes lost during inhibit)
    OutputError,

    // Key Events
    /// Key pressed (keycode 0x00-0xFF)
    KeyDown(u8),
    /// Key repeat indicator (sent repeatedly for held keys in AutoDown mode)
    Repeat,
    /// All keys released (sent when last key in UpDown mode is released)
    AllUp,
    /// Prefix indicating next byte is keycode for key already down
    PrefixKeyDown(u8),
}

impl LK201Response {
    /// Serialize this response to bytes for transmission
    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            LK201Response::PowerUpSelfTest {
                keyboard_id_firmware,
                keyboard_id_hardware,
                error,
                keycode,
            } => {
                vec![
                    *keyboard_id_firmware,
                    *keyboard_id_hardware,
                    (*error).into(),
                    *keycode,
                ]
            }
            LK201Response::KeyboardId {
                firmware_id,
                hardware_id,
            } => {
                vec![*firmware_id, *hardware_id]
            }
            LK201Response::ModeChangeAck => vec![0xBA],
            LK201Response::KeyboardLockAck => vec![0xB7],
            LK201Response::TestModeAck => vec![0xB8],
            LK201Response::InputError => vec![0xB6],
            LK201Response::OutputError => vec![0xB5],
            LK201Response::KeyDown(keycode) => vec![*keycode],
            LK201Response::Repeat => vec![0xB4],
            LK201Response::AllUp => vec![0xB3],
            LK201Response::PrefixKeyDown(keycode) => vec![0xB9, *keycode],
        }
    }
}

/// Keyboard type IDs (returned in byte 1 of KeyboardId response)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardType {
    /// LK201 keyboard
    LK201 = 1,
    /// LK401 keyboard (has ALT keys)
    LK401 = 2,
    /// LK443 keyboard
    LK443 = 3,
    /// LK421 keyboard
    LK421 = 4,
}

impl KeyboardType {
    pub fn from_id(id: u8) -> Option<Self> {
        match id {
            1 => Some(KeyboardType::LK201),
            2 => Some(KeyboardType::LK401),
            3 => Some(KeyboardType::LK443),
            4 => Some(KeyboardType::LK421),
            _ => None,
        }
    }
}

/// Power-up self test error codes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerUpError {
    /// No error detected
    NoError,
    /// Key was down during power-up
    KeyDownError,
    /// Keyboard failure on power-up test
    PowerError,
    /// Unknown error code
    Unknown(u8),
}

impl From<u8> for PowerUpError {
    fn from(byte: u8) -> Self {
        match byte {
            0x00 => PowerUpError::NoError,
            0x3D => PowerUpError::KeyDownError,
            0x3E => PowerUpError::PowerError,
            other => PowerUpError::Unknown(other),
        }
    }
}

impl From<PowerUpError> for u8 {
    fn from(error: PowerUpError) -> u8 {
        match error {
            PowerUpError::NoError => 0x00,
            PowerUpError::KeyDownError => 0x3D,
            PowerUpError::PowerError => 0x3E,
            PowerUpError::Unknown(byte) => byte,
        }
    }
}

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

macro_rules! def_char_keys {
    ($($keycode:literal => $char:literal $( $char_shift:literal )?;)*) => {
        impl LK201Sender {
            pub fn send_char(&self, c: char) -> Result<(), ()> {
                match c {
                $(
                    $char => Ok(_ = (self.send.send($keycode))),
                    $(
                        $char_shift => Ok(_ = (
                            (self.send.send(0xae), self.send.send($keycode), self.send.send(0xb3))
                        )),
                    )?
                )*
                _ => Err(()),
                }
            }
        }
    };
}

def_char_keys!(
0xbf => '`' '~';
0xc0 => '1' '!';
0xc5 => '2' '@';
0xcb => '3' '#';
0xd0 => '4' '$';
0xd6 => '5' '%';
0xdb => '6' '^';
0xe0 => '7' '&';
0xe5 => '8' '*';
0xea => '9' '(';
0xef => '0' ')';
0xf9 => '-' '_';
0xf5 => '=' '+';
0xc1 => 'q' 'Q';
0xc6 => 'w' 'W';
0xcc => 'e' 'E';
0xd1 => 'r' 'R';
0xd7 => 't' 'T';
0xdc => 'y' 'Y';
0xe1 => 'u' 'U';
0xe6 => 'i' 'I';
0xeb => 'o' 'O';
0xf0 => 'p' 'P';

0xfa => '[' '{';
0xf6 => ']' '}';
0xf7 => '\\' '|';

0xc2 => 'a' 'A';
0xc7 => 's' 'S';
0xcd => 'd' 'D';
0xd2 => 'f' 'F';
0xd8 => 'g' 'G';
0xdd => 'h' 'H';
0xe2 => 'j' 'J';
0xe7 => 'k' 'K';
0xec => 'l' 'L';
0xf2 => ';' ':';
0xfb => '\'' '"';

0xc3 => 'z' 'Z';
0xc8 => 'x' 'X';
0xce => 'c' 'C';
0xd3 => 'v' 'V';
0xd9 => 'b' 'B';
0xde => 'n' 'N';
0xe3 => 'm' 'M';
0xc9 => '<' '>';
0xe8 => ',';
0xed => '.';
0xf3 => '/' '?';

0xd4 => ' ';
);

pub struct LK201 {
    recv: mpsc::Receiver<u8>,
    send: mpsc::Sender<u8>,
    kbd_queue: VecDeque<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SpecialKey {
    Kp0 = 0x92,
    KpPeriod = 0x94,
    KpEnter = 0x95,
    Kp1 = 0x96,
    Kp2 = 0x97,
    Kp3 = 0x98,
    Kp4 = 0x99,
    Kp5 = 0x9a,
    Kp6 = 0x9b,
    KpComma = 0x9c,
    Kp7 = 0x9d,
    Kp8 = 0x9e,
    Kp9 = 0x9f,
    KpHyphen = 0xa0,
    KpPf1 = 0xa1,
    KpPf2 = 0xa2,
    KpPf3 = 0xa3,
    KpPf4 = 0xa4,
    Delete = 0xbc,
    Return = 0xbd,
    Tab = 0xbe,
    Lock = 0xb0,
    Meta = 0xb1,
    Shift = 0xae,
    Ctrl = 0xaf,
    Left = 0xa7,
    Right = 0xa8,
    Down = 0xa9,
    Up = 0xaa,
    RShift = 0xab,
    Find = 0x8a,
    InsertHere = 0x8b,
    Remove = 0x8c,
    Select = 0x8d,
    PrevScreen = 0x8e,
    NextScreen = 0x8f,
    F1 = 0x56,
    F2 = 0x57,
    F3 = 0x58,
    F4 = 0x59,
    F5 = 0x5a,
    F6 = 0x64,
    F7 = 0x65,
    F8 = 0x66,
    F9 = 0x67,
    F10 = 0x68,
    F11 = 0x71,
    F12 = 0x72,
    F13 = 0x73,
    F14 = 0x74,
    Help = 0x7c,
    Menu = 0x7d,
    F17 = 0x80,
    F18 = 0x81,
    F19 = 0x82,
    F20 = 0x83,
}

impl LK201 {
    pub fn new(send: mpsc::Sender<u8>, recv: mpsc::Receiver<u8>) -> Self {
        Self {
            send,
            recv,
            kbd_queue: VecDeque::new(),
        }
    }

    pub fn sender(&self) -> LK201Sender {
        LK201Sender::new(self.send.clone())
    }

    pub fn tick(&mut self) {
        // Accumulate incoming bytes
        let mut received = false;
        while let Ok(byte) = self.recv.try_recv() {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_parse(input: &[u8], expected: LK201Command) {
        let queue = VecDeque::from_iter(input.iter().copied());
        let command = LK201Command::try_from(&queue).unwrap();
        assert_eq!(command, expected);
        assert_eq!(input.len(), command.len());
    }

    #[test]
    fn test_mode_commands() {
        // 0A 80: division 1, autodown, register 0
        test_parse(
            &[0x0A, 0x80],
            LK201Command::SetModeWithAutoRepeat {
                division: Division(1),
                mode: KeyMode::AutoDown,
                register: AutoRepeatRegister(0),
            },
        );

        // 12 81: division 2, autodown, register 1
        test_parse(
            &[0x12, 0x81],
            LK201Command::SetModeWithAutoRepeat {
                division: Division(2),
                mode: KeyMode::AutoDown,
                register: AutoRepeatRegister(1),
            },
        );

        // 72 82: division 14, autodown, register 2
        test_parse(
            &[0x72, 0x82],
            LK201Command::SetModeWithAutoRepeat {
                division: Division(14),
                mode: KeyMode::AutoDown,
                register: AutoRepeatRegister(2),
            },
        );

        // A2: division 4, autodown, no register parameter
        test_parse(
            &[0xA2],
            LK201Command::SetMode {
                mode: KeyMode::AutoDown,
                division: Division(4),
            },
        );
    }

    #[test]
    fn test_autorepeat_commands() {
        // 78 64 9E: register 0, timeout 0x64, rate 0x9E
        test_parse(
            &[0x78, 0x64, 0x9E],
            LK201Command::SetAutoRepeat {
                register: AutoRepeatRegister(0),
                timeout: 0x64,
                rate: 0x1E,
            },
        );

        // 7A 64 9E: register 1, timeout 0x64, rate 0x9E
        test_parse(
            &[0x7A, 0x64, 0x9E],
            LK201Command::SetAutoRepeat {
                register: AutoRepeatRegister(1),
                timeout: 0x64,
                rate: 0x1E,
            },
        );

        // 7C 64 9E: register 2, timeout 0x64, rate 0x9E
        test_parse(
            &[0x7C, 0x64, 0x9E],
            LK201Command::SetAutoRepeat {
                register: AutoRepeatRegister(2),
                timeout: 0x64,
                rate: 0x1E,
            },
        );
    }

    #[test]
    fn test_led_commands() {
        // LED enable and disable with named combinations
        test_parse(&[0x13, 0x84], LK201Command::LedEnable(Led::new(0x84)));
        test_parse(&[0x11, 0x88], LK201Command::LedDisable(Led::new(0x88)));
        test_parse(&[0x13, 0x8F], LK201Command::LedEnable(Led::new(0x8F)));
        test_parse(&[0x13, 0x81], LK201Command::LedEnable(Led::new(0x81)));

        // LED bitmask combinations (any combination of bits 0-3 is valid)
        test_parse(&[0x13, 0x80], LK201Command::LedEnable(Led::new(0x80))); // No LEDs
        test_parse(&[0x13, 0x83], LK201Command::LedEnable(Led::new(0x83))); // Wait + Compose
        test_parse(&[0x13, 0x85], LK201Command::LedEnable(Led::new(0x85))); // Wait + Lock
        test_parse(&[0x13, 0x86], LK201Command::LedEnable(Led::new(0x86))); // Compose + Lock
        test_parse(&[0x13, 0x8C], LK201Command::LedEnable(Led::new(0x8C))); // Lock + Hold
        test_parse(&[0x11, 0x8D], LK201Command::LedDisable(Led::new(0x8D))); // Wait + Lock + Hold
    }

    #[test]
    fn test_bell_and_click_commands() {
        // Bell commands
        test_parse(&[0x23, 0x80], LK201Command::BellEnable(Volume(0)));
        test_parse(&[0xA1], LK201Command::BellDisable);
        test_parse(&[0xA7], LK201Command::RingBell);

        // Click commands
        test_parse(&[0x1B, 0x80], LK201Command::KeyClickEnable(Volume(0)));
        test_parse(&[0x99], LK201Command::KeyClickDisable);
        test_parse(&[0xBB], LK201Command::CtrlKeyClickEnable);
        test_parse(&[0xB9], LK201Command::CtrlKeyClickDisable);
    }

    #[test]
    fn test_control_commands() {
        test_parse(&[0xFD], LK201Command::PowerUp);
        test_parse(&[0xAB], LK201Command::RequestId);
        test_parse(
            &[0xE3],
            LK201Command::EnableRepeat {
                division: Division(0),
            },
        );
        test_parse(
            &[0xE1],
            LK201Command::DisableRepeat {
                division: Division(0),
            },
        );
        test_parse(&[0xD9], LK201Command::RepeatToDown);
        test_parse(&[0xD1], LK201Command::TempNoRepeat);
        test_parse(&[0xD3], LK201Command::SetDefaults);
        test_parse(&[0xCB], LK201Command::TestMode);
        test_parse(&[0x8B], LK201Command::Resume);
        test_parse(&[0x89], LK201Command::Inhibit);

        // Note: 0x80 is TestExit but has same bit pattern as SetMode{division:0, mode:Down}
        // So it will be parsed as a mode command
        test_parse(
            &[0x80],
            LK201Command::SetMode {
                mode: KeyMode::Down,
                division: Division(0),
            },
        );
    }

    #[test]
    fn test_unknown_commands() {
        // Test that invalid command bytes are parsed as Unknown
        test_parse(&[0x0D], LK201Command::Unknown(0x0D));
        test_parse(&[0xFF], LK201Command::Unknown(0xFF));
        test_parse(&[0x01], LK201Command::Unknown(0x01));
        test_parse(&[0x55], LK201Command::Unknown(0x55));

        // Test 3-byte unknown command (0xE9 xx xx - possibly division-specific repeat)
        // Bit pattern: 11101001 = division 13, down mode, type=1
        test_parse(
            &[0xE9, 0x12, 0x34],
            LK201Command::Unknown3(0xE9, 0x12, 0x34),
        );

        // Verify Unknown commands return InputError response
        let cmd = LK201Command::Unknown(0x0D);
        let resp = cmd.response().unwrap();
        assert_eq!(resp, LK201Response::InputError);
        assert_eq!(resp.to_bytes(), vec![0xB6]);
    }

    #[test]
    fn test_command_responses() {
        // Commands that return multi-byte responses
        let cmd = LK201Command::PowerUp;
        let resp = cmd.response().unwrap();
        assert_eq!(resp.to_bytes(), vec![0x01, 0x00, 0x00, 0x00]);

        let cmd = LK201Command::RequestId;
        let resp = cmd.response().unwrap();
        assert_eq!(resp.to_bytes(), vec![0x01, 0x01]);

        // Mode commands return ModeChangeAck (0xBA)
        let cmd = LK201Command::SetMode {
            mode: KeyMode::AutoDown,
            division: Division(1),
        };
        let resp = cmd.response().unwrap();
        assert_eq!(resp.to_bytes(), vec![0xBA]);

        let cmd = LK201Command::SetModeWithAutoRepeat {
            mode: KeyMode::AutoDown,
            division: Division(1),
            register: AutoRepeatRegister(0),
        };
        let resp = cmd.response().unwrap();
        assert_eq!(resp.to_bytes(), vec![0xBA]);

        // Repeat control commands
        let cmd = LK201Command::EnableRepeat {
            division: Division(13),
        };
        let resp = cmd.response().unwrap();
        assert_eq!(resp.to_bytes(), vec![0xBA]);

        let cmd = LK201Command::DisableRepeat {
            division: Division(13),
        };
        let resp = cmd.response().unwrap();
        assert_eq!(resp.to_bytes(), vec![0xBA]);

        let cmd = LK201Command::RepeatToDown;
        let resp = cmd.response().unwrap();
        assert_eq!(resp.to_bytes(), vec![0xBA]);

        // Special acks
        let cmd = LK201Command::TestMode;
        let resp = cmd.response().unwrap();
        assert_eq!(resp.to_bytes(), vec![0xB8]);

        let cmd = LK201Command::Inhibit;
        let resp = cmd.response().unwrap();
        assert_eq!(resp.to_bytes(), vec![0xB7]);

        // Error response
        let cmd = LK201Command::Unknown(0xFF);
        let resp = cmd.response().unwrap();
        assert_eq!(resp.to_bytes(), vec![0xB6]);

        // Commands with no response
        assert!(LK201Command::LedEnable(Led::new(0x84)).response().is_none());
        assert!(LK201Command::BellEnable(Volume(4)).response().is_none());
        assert!(LK201Command::KeyClickDisable.response().is_none());
        assert!(LK201Command::SetDefaults.response().is_none());
        assert!(LK201Command::Resume.response().is_none());
    }

    #[test]
    fn test_response_serialization() {
        // Test all response types serialize correctly
        let resp = LK201Response::ModeChangeAck;
        assert_eq!(resp.to_bytes(), vec![0xBA]);

        let resp = LK201Response::KeyboardLockAck;
        assert_eq!(resp.to_bytes(), vec![0xB7]);

        let resp = LK201Response::TestModeAck;
        assert_eq!(resp.to_bytes(), vec![0xB8]);

        let resp = LK201Response::InputError;
        assert_eq!(resp.to_bytes(), vec![0xB6]);

        let resp = LK201Response::OutputError;
        assert_eq!(resp.to_bytes(), vec![0xB5]);

        let resp = LK201Response::Repeat;
        assert_eq!(resp.to_bytes(), vec![0xB4]);

        let resp = LK201Response::AllUp;
        assert_eq!(resp.to_bytes(), vec![0xB3]);

        let resp = LK201Response::KeyDown(0x42);
        assert_eq!(resp.to_bytes(), vec![0x42]);

        let resp = LK201Response::PrefixKeyDown(0x42);
        assert_eq!(resp.to_bytes(), vec![0xB9, 0x42]);
    }

    #[test]
    fn test_full_sequence() {
        // Test parsing a complete initialization sequence
        // Division 1 (letters), autodown, register 0
        test_parse(
            &[0x0A, 0x80],
            LK201Command::SetModeWithAutoRepeat {
                division: Division(1),
                mode: KeyMode::AutoDown,
                register: AutoRepeatRegister(0),
            },
        );

        // Division 3 (Delete key), autodown, register 0
        test_parse(
            &[0x1A, 0x80],
            LK201Command::SetModeWithAutoRepeat {
                division: Division(3),
                mode: KeyMode::AutoDown,
                register: AutoRepeatRegister(0),
            },
        );

        // Division 7 (left/right arrows), autodown, register 0
        test_parse(
            &[0x3A, 0x80],
            LK201Command::SetModeWithAutoRepeat {
                division: Division(7),
                mode: KeyMode::AutoDown,
                register: AutoRepeatRegister(0),
            },
        );

        // Division 8 (up/down arrows), autodown, register 1
        test_parse(
            &[0x42, 0x81],
            LK201Command::SetModeWithAutoRepeat {
                division: Division(8),
                mode: KeyMode::AutoDown,
                register: AutoRepeatRegister(1),
            },
        );

        // Division 9 (E1-E6), autodown, register 2
        test_parse(
            &[0x4A, 0x82],
            LK201Command::SetModeWithAutoRepeat {
                division: Division(9),
                mode: KeyMode::AutoDown,
                register: AutoRepeatRegister(2),
            },
        );

        // Division 11 (F6-F10), autodown, register 2
        test_parse(
            &[0x5A, 0x82],
            LK201Command::SetModeWithAutoRepeat {
                division: Division(11),
                mode: KeyMode::AutoDown,
                register: AutoRepeatRegister(2),
            },
        );

        // Division 12 (F11-F14), autodown, register 2
        test_parse(
            &[0x62, 0x82],
            LK201Command::SetModeWithAutoRepeat {
                division: Division(12),
                mode: KeyMode::AutoDown,
                register: AutoRepeatRegister(2),
            },
        );

        // Division 13 (Help, Do), autodown, register 2
        test_parse(
            &[0x6A, 0x82],
            LK201Command::SetModeWithAutoRepeat {
                division: Division(13),
                mode: KeyMode::AutoDown,
                register: AutoRepeatRegister(2),
            },
        );
    }
}
