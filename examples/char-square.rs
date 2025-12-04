use std::io::Write;

#[derive(PartialEq, Eq, Copy, Clone, Debug)]
pub enum CharacterSet {
    ASCII,
    DECSupplimentalGraphic,
    DECSpecialGraphic,
    DECTechnical,
    UserPreferredSupplimental,
    NRCISOUnitedKingdon,
    DECFinnish,
    ISOFrench,
    DECFrenchCanadian,
    ISOGerman,
    ISOItalian,
    ISONorwegianDanish,
    DECNorwegianDanish,
    DECPortuguese,
    ISOSpanish,
    DECSwedish,
    DECSwiss,
    ISOLatin1Supplimental,
    /// Assumes identifier of ' @'
    Custom,
}

impl CharacterSet {
    pub const fn all() -> &'static [CharacterSet] {
        &[
            CharacterSet::ASCII,
            CharacterSet::DECSupplimentalGraphic,
            CharacterSet::DECSpecialGraphic,
            CharacterSet::DECTechnical,
            CharacterSet::UserPreferredSupplimental,
            CharacterSet::NRCISOUnitedKingdon,
            CharacterSet::DECFinnish,
            CharacterSet::ISOFrench,
            CharacterSet::DECFrenchCanadian,
            CharacterSet::ISOGerman,
            CharacterSet::ISOItalian,
            CharacterSet::ISONorwegianDanish,
            CharacterSet::DECNorwegianDanish,
            CharacterSet::DECPortuguese,
            CharacterSet::ISOSpanish,
            CharacterSet::DECSwedish,
            CharacterSet::DECSwiss,
            CharacterSet::ISOLatin1Supplimental,
            CharacterSet::Custom,
        ]
    }

    pub fn from_str(s: &str) -> Option<CharacterSet> {
        match s.to_ascii_lowercase().as_str() {
            "ascii" => Some(CharacterSet::ASCII),
            "decsupplimentalgraphic" => Some(CharacterSet::DECSupplimentalGraphic),
            "decspecialgraphic" => Some(CharacterSet::DECSpecialGraphic),
            "dectechnical" => Some(CharacterSet::DECTechnical),
            "userpreferredsupplimental" => Some(CharacterSet::UserPreferredSupplimental),
            "nrcisounitedkingdon" => Some(CharacterSet::NRCISOUnitedKingdon),
            "decfinnish" => Some(CharacterSet::DECFinnish),
            "isofrench" => Some(CharacterSet::ISOFrench),
            "decfrenchcanadian" => Some(CharacterSet::DECFrenchCanadian),
            "isogerman" => Some(CharacterSet::ISOGerman),
            "isoitalian" => Some(CharacterSet::ISOItalian),
            "isonorwegiandanish" => Some(CharacterSet::ISONorwegianDanish),
            "decnorwegiandanish" => Some(CharacterSet::DECNorwegianDanish),
            "decportuguese" => Some(CharacterSet::DECPortuguese),
            "isospanish" => Some(CharacterSet::ISOSpanish),
            "decswedish" => Some(CharacterSet::DECSwedish),
            "decswiss" => Some(CharacterSet::DECSwiss),
            "isolatin1supplimental" => Some(CharacterSet::ISOLatin1Supplimental),
            "decspcialgraphic" => Some(CharacterSet::DECSpecialGraphic),
            "custom" => Some(CharacterSet::Custom),
            _ => None,
        }
    }

    pub const fn select_g<'a>(&self, n: u8, select_output: &'a mut [u8]) -> &'a [u8] {
        select_output[0] = 0x1b;
        match (n, *self) {
            (0, CharacterSet::ISOLatin1Supplimental) => unreachable!(),
            (1, CharacterSet::ISOLatin1Supplimental) => select_output[1] = b'-',
            (2, CharacterSet::ISOLatin1Supplimental) => select_output[1] = b'.',
            (3, CharacterSet::ISOLatin1Supplimental) => select_output[1] = b'/',
            (0, _) => select_output[1] = b'(',
            (1, _) => select_output[1] = b')',
            (2, _) => select_output[1] = b'*',
            (3, _) => select_output[1] = b'+',
            _ => unreachable!(),
        }
        let select = self.select();
        select_output[2] = select[0];
        if select.len() > 1 {
            select_output[3] = select[1];
        }
        select_output.split_at(select.len() + 2).0
    }

    const fn select(&self) -> &'static [u8] {
        match self {
            // 94-Character Sets
            // ASCII: B
            CharacterSet::ASCII => b"B",
            // DEC Supplemental Graphic: %5
            CharacterSet::DECSupplimentalGraphic => b"%5",
            // DEC Special Graphics: 0
            CharacterSet::DECSpecialGraphic => b"0",
            // DEC Technical: >
            CharacterSet::DECTechnical => b">",
            // User-preferred supplemental: <
            CharacterSet::UserPreferredSupplimental => b"(<",
            // ISO United Kingdom: A
            CharacterSet::NRCISOUnitedKingdon => b"A",
            // DEC Finnish: 5
            CharacterSet::DECFinnish => b"5",
            // ISO French: R
            CharacterSet::ISOFrench => b"R",
            // DEC French Canadian: 9
            CharacterSet::DECFrenchCanadian => b"9",
            // ISO German: K
            CharacterSet::ISOGerman => b"K",
            // ISO Italian: Y
            CharacterSet::ISOItalian => b"Y",
            // ISO Norwegian/Danish: '
            CharacterSet::ISONorwegianDanish => b"'",
            // DEC Norwegian/Danish: 6
            CharacterSet::DECNorwegianDanish => b"6",
            // DEC Portuguese: %6
            CharacterSet::DECPortuguese => b"%6",
            // ISO Spanish: Z
            CharacterSet::ISOSpanish => b"Z",
            // DEC Swedish: 7
            CharacterSet::DECSwedish => b"7",
            // DEC Swiss: =
            CharacterSet::DECSwiss => b"=",
            // 96-Character Sets
            // ISO Latin-1 Supplemental: A
            CharacterSet::ISOLatin1Supplimental => b"A",
            // Custom: @
            CharacterSet::Custom => b" @",
        }
    }
}

fn locking_shift_left(n: u8) -> &'static [u8] {
    match n {
        0 => b"\x0f",
        1 => b"\x0e",
        2 => b"\x1bn",
        3 => b"\x1bo",
        _ => unreachable!(),
    }
}

fn single_shift_left(n: u8) -> &'static [u8] {
    match n {
        2 => b"\x1bN",
        3 => b"\x1bO",
        _ => unreachable!(),
    }
}

fn main() {
    let mut selected = Vec::new();
    for arg in std::env::args().skip(1) {
        if let Some(character_set) = CharacterSet::from_str(&arg) {
            selected.push(character_set);
        }
    }
    if selected.is_empty() {
        selected = vec![
            CharacterSet::ASCII,
            CharacterSet::DECSupplimentalGraphic,
            CharacterSet::DECSpecialGraphic,
            CharacterSet::DECTechnical,
        ];
    }

    for (i, character_set) in selected.into_iter().enumerate() {
        println!();
        print!(
            "Character set: {:?}, Select: {:?}",
            character_set,
            character_set.select()
        );
        std::io::stdout()
            .write_all(character_set.select_g(i as u8, &mut [0; 4]))
            .unwrap();
        std::io::stdout()
            .write_all(locking_shift_left(i as u8))
            .unwrap();
        for i in 32..128 {
            if i % 16 == 0 {
                println!();
            }
            std::io::stdout().write_all(&[i as u8]).unwrap();
            // std::io::stdout().write_all(b"[[[").unwrap();
        }
        std::io::stdout().write_all(locking_shift_left(0)).unwrap();
    }
    println!();
}
