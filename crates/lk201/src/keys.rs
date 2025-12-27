#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    CharShift(char, char),
    Special(SpecialKey),
}

impl Key {
    pub const fn scancode(&self) -> Option<u8> {
        Some(match self {
            Key::Char(c) => {
                if let Some((keycode, _)) = Key::char_to_keycode(*c) {
                    keycode
                } else {
                    return None;
                }
            }
            Key::CharShift(c, _) => {
                if let Some((keycode, _)) = Key::char_to_keycode(*c) {
                    keycode
                } else {
                    return None;
                }
            }
            Key::Special(special) => *special as u8,
        })
    }
}

macro_rules! def_keys {
    (char = {
        $(
            $keycode:literal => ( $char:literal $( $char_shift:literal )? );
        )*
    }
    special = {
        $(
            $keycode_special:literal => $special_key:ident;
        )*
    }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[repr(u8)]
        pub enum SpecialKey {
            $(
                $special_key = $keycode_special,
            )*
        }

        pub const ALL_KEYS: [Option<Key>; 256] = {
            let mut keys = [None; 256];
            let mut i = 0;
            while i < 256 {
                keys[i] = Key::from_keycode(i as u8);
                i += 1;
            }
            keys
        };

        impl Key {
            pub const fn from_keycode(keycode: u8) -> Option<Self> {
                match keycode {
                    $(
                        $keycode => {
                            let slice = &[$( $char_shift )?];
                            if let Some(b) = slice.first() {
                                Some(Key::CharShift($char, *b))
                            } else {
                                Some(Key::Char($char))
                            }
                        }
                    )*
                    $(
                        $keycode_special => Some(Key::Special(SpecialKey::$special_key)),
                    )*
                    _ => None,
                }
            }

            pub const fn char_to_keycode(c: char) -> Option<(u8, bool)> {
                match c {
                    $(
                        $char => Some(($keycode, false)),
                        $( $char_shift => Some(($keycode, true)), )*
                    )*
                    _ => None,
                }
            }
        }
    };
}

def_keys!(
    char = {
        0xbf => ('`' '~');
        0xc0 => ('1' '!');
        0xc5 => ('2' '@');
        0xcb => ('3' '#');
        0xd0 => ('4' '$');
        0xd6 => ('5' '%');
        0xdb => ('6' '^');
        0xe0 => ('7' '&');
        0xe5 => ('8' '*');
        0xea => ('9' '(');
        0xef => ('0' ')');
        0xf9 => ('-' '_');
        0xf5 => ('=' '+');
        0xc1 => ('q' 'Q');
        0xc6 => ('w' 'W');
        0xcc => ('e' 'E');
        0xd1 => ('r' 'R');
        0xd7 => ('t' 'T');
        0xdc => ('y' 'Y');
        0xe1 => ('u' 'U');
        0xe6 => ('i' 'I');
        0xeb => ('o' 'O');
        0xf0 => ('p' 'P');

        0xfa => ('[' '{');
        0xf6 => (']' '}');
        0xf7 => ('\\' '|');

        0xc2 => ('a' 'A');
        0xc7 => ('s' 'S');
        0xcd => ('d' 'D');
        0xd2 => ('f' 'F');
        0xd8 => ('g' 'G');
        0xdd => ('h' 'H');
        0xe2 => ('j' 'J');
        0xe7 => ('k' 'K');
        0xec => ('l' 'L');
        0xf2 => (';' ':');
        0xfb => ('\'' '"');

        0xc3 => ('z' 'Z');
        0xc8 => ('x' 'X');
        0xce => ('c' 'C');
        0xd3 => ('v' 'V');
        0xd9 => ('b' 'B');
        0xde => ('n' 'N');
        0xe3 => ('m' 'M');
        0xc9 => ('<' '>');
        0xe8 => (',');
        0xed => ('.');
        0xf3 => ('/' '?');

        0xd4 => (' ');
    }
    special = {
        0x92 => Kp0;
        0x94 => KpPeriod;
        0x95 => KpEnter;
        0x96 => Kp1;
        0x97 => Kp2;
        0x98 => Kp3;
        0x99 => Kp4;
        0x9a => Kp5;
        0x9b => Kp6;
        0x9c => KpComma;
        0x9d => Kp7;
        0x9e => Kp8;
        0x9f => Kp9;
        0xa0 => KpHyphen;
        0xa1 => KpPf1;
        0xa2 => KpPf2;
        0xa3 => KpPf3;
        0xa4 => KpPf4;
        0xbc => Delete;
        0xbd => Return;
        0xbe => Tab;
        0xb0 => Lock;
        0xb1 => Meta;
        0xae => Shift;
        0xaf => Ctrl;
        0xa7 => Left;
        0xa8 => Right;
        0xa9 => Down;
        0xaa => Up;
        0xab => RShift;
        0x8a => Find;
        0x8b => InsertHere;
        0x8c => Remove;
        0x8d => Select;
        0x8e => PrevScreen;
        0x8f => NextScreen;

        0x56 => F1;
        0x57 => F2;
        0x58 => F3;
        0x59 => F4;
        0x5a => F5;
        0x64 => F6;
        0x65 => F7;
        0x66 => F8;
        0x67 => F9;
        0x68 => F10;
        0x71 => F11;
        0x72 => F12;
        0x73 => F13;
        0x74 => F14;
        0x7c => Help;
        0x7d => Menu;
        0x80 => F17;
        0x81 => F18;
        0x82 => F19;
        0x83 => F20;
    }
);
