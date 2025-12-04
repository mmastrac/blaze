use core::fmt;
use std::ops;

use clap::Parser;

#[derive(clap::ValueEnum, Clone, Copy)]
enum Style {
    Digits,
    Boxed,
    Fill,
}

#[derive(clap::Parser)]
struct Args {
    #[clap(long, value_parser)]
    style: Style,
}

/// 5x3 digits
const DIGITS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111], // 0
    [0b010, 0b010, 0b010, 0b010, 0b010], // 1
    [0b111, 0b001, 0b111, 0b100, 0b111], // 2
    [0b111, 0b001, 0b111, 0b001, 0b111], // 3
    [0b101, 0b101, 0b111, 0b001, 0b001], // 4
    [0b111, 0b100, 0b111, 0b001, 0b111], // 5
    [0b111, 0b100, 0b111, 0b101, 0b111], // 6
    [0b111, 0b001, 0b001, 0b001, 0b001], // 7
    [0b111, 0b101, 0b111, 0b101, 0b111], // 8
    [0b111, 0b101, 0b111, 0b001, 0b111], // 9
];

/// 16x10 characters
struct Character {
    data: [[bool; 10]; 16],
    height: u8,
    width: u8,
}

impl fmt::Debug for Character {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Character {{\n")?;
        for row in 0..self.height {
            write!(f, "    ")?;
            for col in 0..self.width {
                write!(
                    f,
                    "{}",
                    if self.data[row as usize][col as usize] {
                        "X"
                    } else {
                        "."
                    }
                )?;
            }
            write!(f, "\n")?;
        }
        write!(f, "}}\n")
    }
}

impl Character {
    fn fill(width: u8, height: u8) -> Self {
        let mut data = [[false; 10]; 16];
        for i in 0..width {
            for j in 0..height {
                data[j as usize][i as usize] = true;
            }
        }
        Character {
            data,
            width,
            height,
        }
    }

    fn boxed(width: u8, height: u8) -> Self {
        let mut data = [[false; 10]; 16];
        for row in 0..height {
            data[row as usize][0] = true;
            data[row as usize][width as usize - 1] = true;
        }
        for col in 0..width {
            data[0][col as usize] = true;
            data[height as usize - 1][col as usize] = true;
        }
        Character {
            data,
            width,
            height,
        }
    }

    fn digits(char: u8, width: u8, height: u8) -> Self {
        let mut data = [[false; 10]; 16];
        let digit0 = char % 10;
        let digit1 = char / 10;
        // Left digit printed at col 0, row 1
        for row in 0..5 {
            for col in 0..3 {
                let pixel = DIGITS[digit1 as usize][row as usize] & (1 << (3 - col - 1)) != 0;
                data[(1 + row) as usize][col as usize] = pixel;
            }
        }
        // Right digit printed at col width - 1, row height - 1
        for row in 0..5 {
            for col in 0..3 {
                let pixel = DIGITS[digit0 as usize][row as usize] & (1 << (3 - col - 1)) != 0;
                data[((height - 6) + row) as usize][((width - 3) + col) as usize] = pixel;
            }
        }
        Character {
            data,
            width,
            height,
        }
    }

    fn encode(&self) -> String {
        // 10 columns, 3 groups of 6 bits
        let mut sixel = [[0_u8; 10]; 3];
        for col in 0..10 {
            for row in 0..16 {
                if self.data[row][col] {
                    sixel[row / 6][col] |= 1 << (row % 6);
                }
            }
        }
        let mut s = String::new();

        for group in 0..3 {
            if group > 0 {
                s.push('/');
            }
            // Find the last non-zero column
            if let Some(last_col) = sixel[group].iter().rposition(|&x| x != 0) {
                for col in 0..=last_col {
                    s.push((sixel[group][col] + b'?') as char);
                }
            }
        }
        s.push(';');
        s
    }
}

impl ops::Deref for Character {
    type Target = [[bool; 10]; 16];
    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl ops::DerefMut for Character {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.data
    }
}

fn encode_font(width: u8, height: u8) -> String {
    // The 96-char soft font doesn't seem to work if you don't load a 94-char
    // soft font first.

    // DCS Pfn ; Pcn; Pe; Pcmw; Pss; Pt; Pcmh; Pcss
    // Pfn = 1 (font 1 for each session)
    // Pcn = 0 (load char 1)
    // Pe = 1 (erase all chars being reloaded)
    // Pcmw = width
    // Pss = 1/2, 11/12, 21/22
    // Pt = 2 (full cell)
    // Pcmh = height
    // Pcss = 0 (94 char)
    let pss = match width {
        6 => 2,
        10 => 1,
        _ => unreachable!(),
    } + match height {
        8 => 20,
        10 => 10,
        16 => 0,
        _ => unreachable!(),
    };
    format!("1;1;1;{};{};2;{};0{{ @", width, pss, height)
}

pub fn main() {
    let args = Args::parse();

    // 80/132
    for width in [10, 6] {
        // 48/36/25
        for height in [16, 10, 8] {
            println!("Generating {}x{} font...", height, width);
            print!("\x1bP{}", encode_font(width, height));
            for c in 0..94 {
                let c = match args.style {
                    Style::Digits => Character::digits(c, width, height),
                    Style::Boxed => Character::boxed(width, height),
                    Style::Fill => Character::fill(width, height),
                };
                print!("{}", c.encode());
            }
            print!("\x1b\\");
        }
    }

    print!("Done:");

    // Select the font named " @" into GS1
    print!("\x1b) @");
    // Locking Shift Left 1
    print!("\x0e");
    // This will print the first 16 soft font chars
    for c in 0x20..0x30 {
        print!("{}", c as u8 as char);
    }
    // Locking Shift Left 0
    print!("\x0f");

    println!();
}
