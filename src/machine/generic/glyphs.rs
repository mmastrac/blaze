//! Glyph-to-Unicode mapping by fingerprint.
//!
//! The text views need Unicode for each cell, but character codes do not say
//! which glyph a terminal draws: it depends on the machine, the font slot and
//! the screen format. Instead we hash the glyph's pixel lines as the display
//! reads them and look the hash up in a table built by driving each machine
//! with known text (the ignored `glyph_table` tests).
//!
//! Some glyphs are identical for different characters (a small font draws Ò
//! and ò the same). The terminal still stores them as different codes, so
//! those fingerprints get entries qualified by font slot and code, which the
//! lookup tries first.
//!
//! The table format is one entry per line, `<16 hex digits> U+<hex>`, with an
//! optional `@<font>:<code>` qualifier (hex) and `#` comments.

use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Default)]
pub struct GlyphTable {
    map: HashMap<u64, char>,
    qualified: HashMap<(u64, u8, u8), char>,
}

/// A hash of a glyph's pixel lines, or `None` for a blank glyph. The line
/// count is part of the hash, so fonts of different heights never collide.
pub fn fingerprint(lines: &[u16]) -> Option<u64> {
    if lines.iter().all(|&line| line == 0) {
        return None;
    }
    // FNV-1a.
    let mut hash: u64 = 0xCBF2_9CE4_8422_2325;
    let mut add = |byte: u8| {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0100_0000_01B3);
    };
    add(lines.len() as u8);
    for &line in lines {
        add(line as u8);
        add((line >> 8) as u8);
    }
    Some(hash)
}

impl GlyphTable {
    /// Parses one or more tables. Entries in earlier tables win.
    pub fn parse(texts: &[&str]) -> Self {
        let mut table = Self::default();
        for text in texts {
            for line in text.lines() {
                let line = line.split('#').next().unwrap_or("").trim();
                let mut fields = line.split_whitespace();
                let (Some(hash), Some(code)) = (fields.next(), fields.next()) else {
                    continue;
                };
                let (Ok(hash), Some(ch)) = (
                    u64::from_str_radix(hash, 16),
                    code.strip_prefix("U+")
                        .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                        .and_then(char::from_u32),
                ) else {
                    continue;
                };
                match fields.next().and_then(parse_qualifier) {
                    Some((font, code)) => {
                        table.qualified.entry((hash, font, code)).or_insert(ch);
                    }
                    None => {
                        table.map.entry(hash).or_insert(ch);
                    }
                }
            }
        }
        table
    }

    /// The character for a glyph drawn from font slot `font` at `code`: a
    /// space for a blank glyph, `None` when the glyph is not in the table.
    pub fn get(&self, lines: &[u16], font: u8, code: u8) -> Option<char> {
        let hash = fingerprint(lines)?;
        self.qualified
            .get(&(hash, font, code))
            .or_else(|| self.map.get(&hash))
            .copied()
    }

    /// Like `get`, but blank glyphs give a space.
    pub fn get_or_blank(&self, lines: &[u16], font: u8, code: u8) -> Option<char> {
        if fingerprint(lines).is_none() {
            return Some(' ');
        }
        self.get(lines, font, code)
    }
}

fn parse_qualifier(text: &str) -> Option<(u8, u8)> {
    let (font, code) = text.strip_prefix('@')?.split_once(':')?;
    Some((
        u8::from_str_radix(font, 16).ok()?,
        u8::from_str_radix(code, 16).ok()?,
    ))
}

/// Collects labelled glyphs for a table.
#[derive(Default)]
pub struct GlyphTableBuilder {
    /// The first label for each fingerprint.
    first: BTreeMap<u64, char>,
    /// Every label, by fingerprint, font slot and code.
    labels: BTreeMap<(u64, u8, u8), char>,
    /// Two different labels for the same fingerprint, font slot and code.
    pub conflicts: Vec<(u64, u8, u8, char, char)>,
}

impl GlyphTableBuilder {
    pub fn add(&mut self, lines: &[u16], ch: char, font: u8, code: u8) {
        let Some(hash) = fingerprint(lines) else {
            return;
        };
        self.first.entry(hash).or_insert(ch);
        match self.labels.get(&(hash, font, code)) {
            Some(&old) if old != ch => {
                if !self
                    .conflicts
                    .iter()
                    .any(|&(h, f, c, _, new)| (h, f, c, new) == (hash, font, code, ch))
                {
                    self.conflicts.push((hash, font, code, old, ch));
                }
            }
            Some(_) => {}
            None => {
                self.labels.insert((hash, font, code), ch);
            }
        }
    }

    pub fn len(&self) -> usize {
        self.first.len()
    }

    /// Fingerprints labelled with more than one character (at different
    /// codes).
    pub fn ambiguous(&self) -> BTreeSet<u64> {
        let mut chars: BTreeMap<u64, BTreeSet<char>> = BTreeMap::new();
        for (&(hash, _, _), &ch) in &self.labels {
            chars.entry(hash).or_default().insert(ch);
        }
        chars
            .into_iter()
            .filter(|(_, set)| set.len() > 1)
            .map(|(hash, _)| hash)
            .collect()
    }

    pub fn to_text(&self, header: &str) -> String {
        let ambiguous = self.ambiguous();
        let mut out = String::new();
        for line in header.lines() {
            out.push_str(&format!("# {line}\n"));
        }
        for (hash, ch) in &self.first {
            out.push_str(&format!("{hash:016x} U+{:04X}\n", *ch as u32));
            if ambiguous.contains(hash) {
                for (&(_, font, code), label) in
                    self.labels.range((*hash, 0, 0)..=(*hash, u8::MAX, u8::MAX))
                {
                    out.push_str(&format!(
                        "{hash:016x} U+{:04X} @{font:X}:{code:02X}\n",
                        *label as u32
                    ));
                }
            }
        }
        out
    }
}
