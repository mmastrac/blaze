//! DEC-to-unicode mapping for various character sets.

pub fn map_char(ch: u16) -> Option<char> {
    match ch {
        0x00 => Some(' '),
        0x01 => Some('◆'),
        0x02 => Some('░'),
        0x03 => Some('␉'),
        0x04 => Some('␌'),
        0x05 => Some('␍'),
        0x06 => Some('␊'),
        0x07 => Some('°'),
        0x0b => Some('╯'),
        0x0c => Some('╮'),
        0x0d => Some('╭'),
        0x0e => Some('╰'),
        0x10 => Some('⎺'),
        0x11 => Some('⎻'),
        0x12 => Some('─'),
        0x13 => Some('⎽'),
        0x19 => Some('│'),
        0x20..0x7e => Some(ch as u8 as char),
        0x198 => Some('█'),
        0xa9 => Some('©'),
        0xd7 => Some('×'), // middle x
        0x120 => Some('1'),
        0x121 => Some('√'),
        0x138 => Some('2'),

        // These are scattered through the font with no particular pattern
        0x909 => Some('s'),
        0x109 => Some('r'),
        0x90a => Some('u'),
        0x10a => Some('t'),
        0x90b => Some('z'),
        0x10b => Some('y'),
        0x90c => Some('C'),
        0x10c => Some('A'),
        0x90d => Some('H'),
        0x10d => Some('F'),
        0x939 => Some('å'),
        0x139 => Some('é'),
        0x952 => Some('d'),
        0x954 => Some('g'),
        0x154 => Some('e'),
        0x955 => Some('i'),
        0x155 => Some('h'),
        0x96d => Some('l'),
        0x16d => Some('k'),
        0x975 => Some('n'),
        0x175 => Some('m'),
        0x97f => Some('p'),
        0x17f => Some('o'),

        0x99b => Some('L'),
        0x19b => Some('K'),
        0x99c => Some('S'),
        0x19c => Some('P'),
        0x99d => Some('W'),
        0x19d => Some('V'),
        0x99e => Some('ó'),

        // Also shows up as ae??
        // This seems to be the one and only one context-specific decoding. In danish, we get ae,
        // in norweigian we get B.
        0x19e => Some('B'),

        _ => None,
    }
}
