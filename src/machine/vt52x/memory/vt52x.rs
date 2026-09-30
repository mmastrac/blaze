use super::RAM;

const REG_ROW_FORMAT: u8 = 0xB8;
const REG_ROW_FORMAT_2: u8 = 0xBC;

pub(super) fn window_format(ram: &RAM, lower: bool) -> u8 {
    ram.regs.regs[if lower {
        REG_ROW_FORMAT_2
    } else {
        REG_ROW_FORMAT
    } as usize]
}

pub(crate) fn font_column(video_format: u8, attr: u8) -> u8 {
    match attr & 3 {
        sel @ (1 | 2) => {
            let session = match (video_format >> 1) & 3 {
                0 => 4,
                s => s,
            };
            session * 16 + (sel - 1) * 8
        }
        _ => (video_format & 1) * 8,
    }
}

pub(super) fn rom_bank(p1: u8) -> u8 {
    (p1 >> 4) & 0b111
}

pub(super) const COMM_TX_BYTE_INSTRUCTIONS: usize = 1000;
