//! Builds `glyphs.txt`: boots each ROM in each screen format, prints known
//! text in several character sets, and records the fingerprint of every glyph
//! it draws. Run with
//! `GLYPHS_WRITE=1 cargo test --release -- --ignored glyph_table`.

use std::fs;

use i8051::Cpu;

use crate::machine::generic::glyphs::{GlyphTable, GlyphTableBuilder, fingerprint};
use crate::machine::generic::keyboard::KeyCap;
use crate::machine::generic::keyboard::KeyboardInput;
use crate::machine::generic::keyboard::lk401_input::Lk401Input;
use crate::machine::vt52x::System;
use crate::machine::vt52x::framebuffer::{for_each_cell, glyph_lines};
use crate::machine::vt52x::memory::TextCell;

/// DEC Special Graphics (`ESC ( 0`), 0x60-0x7E.
const SPECIAL_GRAPHICS: &str = "◆▒␉␌␍␊°±␤␋┘┐┌└┼⎺⎻─⎼⎽├┤┴┬│≤≥π≠£·";

/// DEC Technical (`ESC ( >`), 0x21-0x7E. `\0` marks the undefined codes,
/// which draw the reversed question mark.
const TECHNICAL: &str = "⎷┌─⌠⌡│⎡⎣⎤⎦⎛⎝⎞⎠⎨⎬⎲⎳╲╱⌝⌟⟩\0\0\0\0≤≠≥∫∴∝∞÷Δ∇ΦΓ∼≃Θ×Λ⇔⇒≡ΠΨ\0Σ\0\0√ΩΞΥ⊂⊃∩∪∧∨¬αβχδεφγηιθκλ\0ν∂πψρστ\0ƒωξυζ←↑→↓";

/// Glyphs only Set-Up and the framed-window title rows draw, by character
/// code in the main font.
const VT520_SETUP_SYMBOLS: &[(u8, char)] = &[
    (0x01, '□'),
    (0x02, '☑'),
    (0x03, '○'),
    (0x04, '◉'),
    (0x05, '▶'),
    (0x06, '▲'),
    (0x07, '▼'),
    // Framed windows: the session tab (a monitor with "S", then its number)
    // and the striped title bar.
    (0x08, 'S'),
    (0x09, '1'),
    (0x0A, '2'),
    (0x0B, '3'),
    (0x0C, '4'),
    (0x0D, '≡'),
    (0x0E, '♪'),
    (0x10, '█'),
];

struct Model {
    name: &'static str,
    rom: &'static str,
    boot_instructions: usize,
    setup_symbols: &'static [(u8, char)],
}

const MODELS: &[Model] = &[
    Model {
        name: "VT520",
        rom: "roms/vt520/23-010ED-00.bin",
        boot_instructions: 20_000_000,
        setup_symbols: VT520_SETUP_SYMBOLS,
    },
    Model {
        name: "VT525",
        rom: "roms/vt525/23-011ED-00.bin",
        boot_instructions: 25_000_000,
        setup_symbols: VT520_SETUP_SYMBOLS,
    },
    Model {
        name: "VT510",
        rom: "roms/vt510/23-032ED-00.bin",
        boot_instructions: 11_000_000,
        setup_symbols: &[],
    },
];

/// A character set to print: the sequence that selects it, and the bytes and
/// the characters they should draw.
struct Sample {
    select: String,
    codes: Vec<u8>,
    chars: Vec<char>,
}

/// 96-character ISO sets: the final byte that designates the set into G1
/// (after `ESC -`), and the characters at 0xA1-0xFF once G1 is in the upper
/// half (`ESC ~`). `\0` marks codes left out: unassigned, invisible (soft
/// hyphen, direction marks), or added to the standard after these terminals.
const ISO_96: &[(&str, &str)] = &[
    (
        "B",
        "Ą˘Ł¤ĽŚ§¨ŠŞŤŹ\0ŽŻ°ą˛ł´ľśˇ¸šşťź˝žżŔÁÂĂÄĹĆÇČÉĘËĚÍÎĎĐŃŇÓÔŐÖ×ŘŮÚŰÜÝŢßŕáâăäĺćçčéęëěíîďđńňóôőö÷řůúűüýţ˙",
    ), // ISO Latin-2
    (
        "F",
        "‘’£\0\0¦§¨©\0«¬\0\0―°±²³΄΅Ά·ΈΉΊ»Ό½ΎΏΐΑΒΓΔΕΖΗΘΙΚΛΜΝΞΟΠΡ\0ΣΤΥΦΧΨΩΪΫάέήίΰαβγδεζηθικλμνξοπρςστυφχψωϊϋόύώ\0",
    ), // ISO Greek
    (
        "H",
        "\0¢£¤¥¦§¨©×«¬\0®¯°±²³´µ¶·¸¹÷»¼½¾\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0‗אבגדהוזחטיךכלםמןנסעףפץצקרשת\0\0\0\0\0",
    ), // ISO Hebrew
    (
        "L",
        "ЁЂЃЄЅІЇЈЉЊЋЌ\0ЎЏАБВГДЕЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯабвгдежзийклмнопрстуфхцчшщъыьэюя№ёђѓєѕіїјљњћќ§ўџ",
    ), // ISO Latin-Cyrillic
    (
        "M",
        "¡¢£¤¥¦§¨©ª«¬\0®¯°±²³´µ¶·¸¹º»¼½¾¿ÀÁÂÃÄÅÆÇÈÉÊËÌÍÎÏĞÑÒÓÔÕÖ×ØÙÚÛÜİŞßàáâãäåæçèéêëìíîïğñòóôõö÷øùúûüışÿ",
    ), // ISO Latin-5
];

fn samples() -> Vec<Sample> {
    assert_eq!(SPECIAL_GRAPHICS.chars().count(), 0x7E - 0x60 + 1);
    assert_eq!(TECHNICAL.chars().count(), 0x7E - 0x21 + 1);
    let ascii: Vec<u8> = (0x21..=0x7E).collect();
    let latin1: Vec<u8> = (0xA1..=0xFF).collect();
    let special: Vec<u8> = (0x60..=0x7E).collect();
    let technical: Vec<u8> = (0x21..=0x7E).collect();
    vec![
        Sample {
            select: String::new(),
            chars: ascii.iter().map(|&c| c as char).collect(),
            codes: ascii,
        },
        // The factory setting puts ISO Latin-1 in the upper half.
        Sample {
            select: String::new(),
            chars: latin1.iter().map(|&c| c as char).collect(),
            codes: latin1,
        },
        Sample {
            select: "\x1b(0".into(),
            chars: SPECIAL_GRAPHICS.chars().collect(),
            codes: special,
        },
        Sample {
            select: "\x1b(>".into(),
            chars: TECHNICAL.chars().collect(),
            codes: technical,
        },
    ]
    .into_iter()
    .chain(ISO_96.iter().map(|(set, chars)| {
        assert_eq!(chars.chars().count(), 0xFF - 0xA1 + 1, "{set}");
        Sample {
            select: format!("\x1b-{set}\x1b~"),
            codes: (0xA1..=0xFF).collect(),
            chars: chars.chars().collect(),
        }
    }))
    .collect()
}

fn run(system: &mut System, cpu: &mut Cpu, n: usize) {
    for _ in 0..n {
        system.step(cpu);
    }
}

fn send(system: &mut System, cpu: &mut Cpu, bytes: &[u8]) {
    for chunk in bytes.chunks(256) {
        for &b in chunk {
            system.memory.comm_receive(0, b);
        }
        run(system, cpu, 1_000_000);
    }
}

/// Labels the symbols on screen that no character set reaches, by code.
fn label_symbols(system: &System, model: &Model, table: &mut GlyphTableBuilder) {
    for_each_cell(&system.memory, |_, _, cell, row_lines| {
        if cell.attr & 3 != 0 || cell.soft_glyph.is_some() {
            return;
        }
        if let Some(&(_, ch)) = model.setup_symbols.iter().find(|(c, _)| *c == cell.ch) {
            table.add(
                &glyph_lines(&system.memory, cell, row_lines),
                ch,
                0,
                cell.ch,
            );
        }
    });
}

/// Runs until the screen stops changing, so a sample is fully drawn before
/// it is labelled (the VT510 draws host text more slowly than the VT520).
fn settle(system: &mut System, cpu: &mut Cpu) {
    let snapshot = |system: &System| {
        let mut cells = vec![];
        for_each_cell(&system.memory, |row, column, cell, _| {
            cells.push((row, column, cell.ch, cell.attr));
        });
        cells
    };
    let mut last = snapshot(system);
    for _ in 0..40 {
        run(system, cpu, 500_000);
        let now = snapshot(system);
        if now == last {
            return;
        }
        last = now;
    }
    panic!("screen did not settle");
}

/// The display row where host text starts: framed windows (the VT525's
/// factory setting) put a session tab row and a title bar above it.
fn text_origin(system: &mut System, cpu: &mut Cpu) -> usize {
    const MARKER: &[u8] = b"0123456789";
    send(system, cpu, b"\x1b[H\x1b[2J0123456789");
    settle(system, cpu);
    let mut origin = None;
    let mut found: Vec<(usize, usize, u8)> = vec![];
    for_each_cell(&system.memory, |row, column, cell, _| {
        found.push((row, column, cell.ch));
    });
    for &(row, column, _) in &found {
        if column != 0 || origin.is_some() {
            continue;
        }
        let codes: Vec<u8> = found
            .iter()
            .filter(|&&(r, c, _)| r == row && c < MARKER.len())
            .map(|&(_, _, ch)| ch)
            .collect();
        if codes == MARKER {
            origin = Some(row);
        }
    }
    origin.expect("marker line not found")
}

/// Labels the glyphs on screen: `expected[row - origin][column]`, `\0` for
/// none.
fn collect(system: &System, origin: usize, expected: &[Vec<char>], table: &mut GlyphTableBuilder) {
    for_each_cell(&system.memory, |row, column, cell, row_lines| {
        let Some(&ch) = row
            .checked_sub(origin)
            .and_then(|r| expected.get(r))
            .and_then(|r| r.get(column))
        else {
            return;
        };
        if ch == '\0' || cell.continuation || cell.soft_glyph.is_some() {
            return;
        }
        let lines = glyph_lines(&system.memory, cell, row_lines);
        // GLYPHS_DEBUG=1 lists what each labelled cell drew.
        if std::env::var("GLYPHS_DEBUG").is_ok() {
            eprintln!(
                "label {ch} code {:02X} font {} column {} lines {row_lines} hash {:016x}",
                cell.ch,
                cell.attr & 3,
                cell.font_column,
                crate::machine::generic::glyphs::fingerprint(&lines).unwrap_or(0)
            );
        }
        table.add(&lines, ch, cell.attr & 3, cell.ch);
    });
}

const FORMATS: [(usize, usize); 6] = [
    (80, 24),
    (80, 36),
    (80, 48),
    (132, 24),
    (132, 36),
    (132, 48),
];

/// Boots `model` past the wait loop and sets the screen format.
fn boot(model: &Model, columns: usize, lines: usize) -> (System, Cpu, Lk401Input) {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let rom = fs::read(format!("{manifest_dir}/{}", model.rom)).unwrap();
    let mut system = System::new(rom, None, None, None).unwrap();
    let mut cpu = Cpu::new();
    run(&mut system, &mut cpu, model.boot_instructions);
    let mut keyboard = system.memory.regs.kbd.input();
    keyboard.tap(KeyCap::A);
    run(&mut system, &mut cpu, 2_000_000);
    let deccolm = if columns == 132 { 'h' } else { 'l' };
    send(
        &mut system,
        &mut cpu,
        format!("\x1b[?3{deccolm}\x1b[{lines}*|\x1b[{lines}t").as_bytes(),
    );
    run(&mut system, &mut cpu, 3_000_000);
    (system, cpu, keyboard)
}

fn scan_model(model: &Model, table: &mut GlyphTableBuilder) {
    for (columns, lines) in FORMATS {
        {
            let before = table.len();
            let (mut system, mut cpu, mut keyboard) = boot(model, columns, lines);

            label_symbols(&system, model, table);
            let origin = text_origin(&mut system, &mut cpu);
            for sample in samples() {
                let mut text = format!("\x1b[H\x1b[2J{}", sample.select).into_bytes();
                let mut expected = vec![];
                for (codes, chars) in sample.codes.chunks(40).zip(sample.chars.chunks(40)) {
                    text.extend_from_slice(codes);
                    text.extend_from_slice(b"\r\n");
                    expected.push(chars.to_vec());
                }
                text.extend_from_slice(b"\x1b(B");
                send(&mut system, &mut cpu, &text);
                settle(&mut system, &mut cpu);
                collect(&system, origin, &expected, table);
            }

            // Set-Up draws symbols no character set reaches.
            keyboard.tap(KeyCap::F3);
            run(&mut system, &mut cpu, 4_000_000);
            label_symbols(&system, model, table);
            eprintln!(
                "{} {columns}x{lines}: {} new glyphs",
                model.name,
                table.len() - before
            );
        }
    }
}

#[test]
#[ignore]
fn glyph_table() {
    let mut table = GlyphTableBuilder::default();
    // GLYPHS_MODEL=VT520 scans one model.
    let only = std::env::var("GLYPHS_MODEL").ok();
    for model in MODELS {
        if only.as_deref().is_some_and(|name| name != model.name) {
            continue;
        }
        scan_model(model, &mut table);
    }
    for (hash, font, code, old, new) in &table.conflicts {
        eprintln!("conflict {hash:016x} @{font:X}:{code:02X}: {old} vs {new}");
    }
    eprintln!("{} fingerprints qualified by code", table.ambiguous().len());
    eprintln!("{} glyphs", table.len());
    if std::env::var("GLYPHS_WRITE").is_ok() {
        let text = table.to_text(
            "Glyph fingerprints for the VT510, VT520 and VT525 (see generic/glyphs.rs).\n\
             Generated by `GLYPHS_WRITE=1 cargo test --release -- --ignored glyph_table`.",
        );
        let path = format!(
            "{}/src/machine/vt52x/glyphs.txt",
            env!("CARGO_MANIFEST_DIR")
        );
        fs::write(path, text).unwrap();
    }
}

/// Every glyph slot in font RAM for one screen format: (font name, font
/// slot, code, pixel lines).
fn font_glyphs(system: &System) -> Vec<(String, u8, u8, Vec<u16>)> {
    let memory = &system.memory;
    let row_lines = memory.row_lines(false);
    // (name, attribute font select, font column) for each font slot.
    let fonts: Vec<(&str, u8, u8)> = if memory.model.is_vt51x() {
        vec![("font 0", 0, 0), ("font 1", 1, 0), ("font 2", 2, 0)]
    } else {
        vec![
            ("main", 0, 0),
            ("alternate 1", 1, 16),
            ("alternate 2", 2, 24),
        ]
    };
    let mut out = vec![];
    for (name, select, column) in fonts {
        for code in 0..=255u8 {
            let cell = TextCell {
                ch: code,
                attr: select,
                font_column: column,
                ..TextCell::default()
            };
            out.push((
                name.to_string(),
                select,
                code,
                glyph_lines(memory, &cell, row_lines),
            ));
        }
    }
    out
}

/// Writes an HTML page showing every glyph in every font and format with the
/// character the table maps it to. Run with
/// `GLYPHS_REVIEW=/path/glyphs.html cargo test --release -- --ignored glyph_review`.
#[test]
#[ignore]
fn glyph_review() {
    let Ok(path) = std::env::var("GLYPHS_REVIEW") else {
        return;
    };
    let vision = GlyphTable::parse(&[include_str!("glyphs-vision.txt")]);
    let table = GlyphTable::parse(&[include_str!("glyphs.txt")]);
    let only = std::env::var("GLYPHS_MODEL").ok();
    let mut sections = vec![];
    for model in MODELS {
        if only.as_deref().is_some_and(|name| name != model.name) {
            continue;
        }
        for (columns, lines) in FORMATS {
            let (system, _, _) = boot(model, columns, lines);
            let mut fonts: Vec<(String, Vec<String>)> = vec![];
            for (font, select, code, glyph) in font_glyphs(&system) {
                if fonts.last().is_none_or(|(name, _)| *name != font) {
                    fonts.push((font.clone(), vec![]));
                }
                let (label, source) = match fingerprint(&glyph) {
                    None => ("null".to_string(), ""),
                    Some(_) => match (
                        table.get(&glyph, select, code),
                        vision.get(&glyph, select, code),
                    ) {
                        (Some(ch), _) => (json_string(&ch.to_string()), "e"),
                        (None, Some(ch)) => (json_string(&ch.to_string()), "v"),
                        (None, None) => ("\"\"".to_string(), ""),
                    },
                };
                let lines: Vec<String> = glyph.iter().map(|l| l.to_string()).collect();
                fonts.last_mut().unwrap().1.push(format!(
                    "[{code},{label},[{}],{:?},{select}]",
                    lines.join(","),
                    source
                ));
            }
            let fonts: Vec<String> = fonts
                .iter()
                .map(|(name, glyphs)| format!("[{},[{}]]", json_string(name), glyphs.join(",")))
                .collect();
            sections.push(format!(
                "[{},[{}]]",
                json_string(&format!("{} {columns}x{lines}", model.name)),
                fonts.join(",")
            ));
            eprintln!("{} {columns}x{lines}", model.name);
        }
    }
    let data = format!("[{}]", sections.join(",\n"));
    fs::write(path, REVIEW_PAGE.replace("/*DATA*/", &data)).unwrap();
}

fn json_string(text: &str) -> String {
    let mut out = String::from("\"");
    for ch in text.chars() {
        match ch {
            '"' | '\\' => {
                out.push('\\');
                out.push(ch);
            }
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

const REVIEW_PAGE: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Glyph Map Review</title>
<style>
:root { --bg:#fbfaf7; --fg:#1d1d1b; --muted:#6b6a64; --ink:#1d1d1b; --tile:#ffffff;
  --line:#e2e0d8; --miss:#c2410c; --missbg:#fff1e8; --vis:#1d4ed8; --visbg:#eaf0ff; }
@media (prefers-color-scheme: dark) { :root:not([data-theme="light"]) {
  --bg:#161614; --fg:#ecebe6; --muted:#9a998f; --ink:#f3c969; --tile:#1f1f1c;
  --line:#34332e; --miss:#fb923c; --missbg:#3a2415; --vis:#93b4ff; --visbg:#1a2340; } }
:root[data-theme="dark"] { --bg:#161614; --fg:#ecebe6; --muted:#9a998f; --ink:#f3c969;
  --tile:#1f1f1c; --line:#34332e; --miss:#fb923c; --missbg:#3a2415; --vis:#93b4ff; --visbg:#1a2340; }
body { margin:0; padding:24px 16px; background:var(--bg); color:var(--fg);
  font:14px/1.45 system-ui, sans-serif; }
h1 { font-size:20px; margin:0 0 4px; } h2 { font-size:16px; margin:28px 0 8px; }
h3 { font-size:13px; color:var(--muted); font-weight:600; margin:14px 0 6px; }
.note { color:var(--muted); margin:0 0 12px; max-width:70ch; }
.controls { display:flex; gap:12px; flex-wrap:wrap; align-items:center; margin:12px 0; }
.grid { display:grid; grid-template-columns:repeat(auto-fill, minmax(46px, 1fr)); gap:4px; }
.g { background:var(--tile); border:1px solid var(--line); border-radius:4px; padding:3px 2px 2px;
  display:flex; flex-direction:column; align-items:center; }
.g.miss { background:var(--missbg); border-color:var(--miss); }
.g canvas { image-rendering:pixelated; }
.g .c { font-size:15px; height:20px; line-height:20px; }
.g.miss .c { color:var(--miss); font-size:11px; }
.g.vis { background:var(--visbg); border-color:var(--vis); } .g.vis .c { color:var(--vis); }
.g .n { font:10px ui-monospace, monospace; color:var(--muted); }
.hide-blank .blank, .only-miss .g:not(.miss) { display:none; }
</style></head><body>
<h1>Glyph Map Review</h1>
<p class="note">Every glyph slot in font RAM for each ROM and screen format, drawn from the emulator,
with the character its fingerprint maps to. Orange tiles are glyphs the table does not know; blue
tiles were identified from the image by a vision model rather than by the terminal.
The number under each glyph is its code in that font.</p>
<div class="controls">
<label><input type="checkbox" id="blank" checked> Hide blank glyphs</label>
<label><input type="checkbox" id="miss"> Only unmapped glyphs</label>
<select id="section"></select>
</div>
<div id="out"></div>
<script>
const DATA = /*DATA*/;
const out = document.getElementById('out');
const sel = document.getElementById('section');
DATA.forEach(([name], i) => sel.add(new Option(name, i)));
const ink = () => getComputedStyle(document.documentElement).getPropertyValue('--ink').trim();
function draw(canvas, lines) {
  const scale = 3, w = 10, h = lines.length;
  canvas.width = w * scale; canvas.height = h * scale;
  const ctx = canvas.getContext('2d'); ctx.fillStyle = ink();
  lines.forEach((bits, y) => { for (let x = 0; x < w; x++) if (bits >> x & 1) ctx.fillRect(x * scale, y * scale, scale, scale); });
}
function render() {
  out.innerHTML = '';
  const [name, fonts] = DATA[sel.value];
  const h2 = document.createElement('h2'); h2.textContent = name; out.append(h2);
  for (const [font, glyphs] of fonts) {
    const h3 = document.createElement('h3');
    const known = glyphs.filter(g => g[1] && g[3] !== 'v').length, vis = glyphs.filter(g => g[3] === 'v').length,
      miss = glyphs.filter(g => g[1] === '').length;
    h3.textContent = `${font}: ${known} from the terminal, ${vis} from images, ${miss} unmapped`; out.append(h3);
    const grid = document.createElement('div'); grid.className = 'grid';
    for (const [code, label, lines, source] of glyphs) {
      const g = document.createElement('div');
      g.className = 'g' + (label === null ? ' blank' : label === '' ? ' miss' : source === 'v' ? ' vis' : '');
      const c = document.createElement('canvas'); draw(c, lines);
      const l = document.createElement('div'); l.className = 'c';
      l.textContent = label === null ? '' : label === '' ? '?' : label;
      const n = document.createElement('div'); n.className = 'n';
      n.textContent = code.toString(16).toUpperCase().padStart(2, '0');
      g.append(c, l, n); grid.append(g);
    }
    out.append(grid);
  }
}
const apply = () => { document.body.classList.toggle('hide-blank', document.getElementById('blank').checked);
  document.body.classList.toggle('only-miss', document.getElementById('miss').checked); };
document.getElementById('blank').onchange = apply; document.getElementById('miss').onchange = apply;
sel.onchange = render; apply(); render();
</script></body></html>
"#;
