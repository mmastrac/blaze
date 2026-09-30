use super::RAM;

pub(super) fn window_format(ram: &RAM) -> u8 {
    let r = &ram.regs.regs;
    r[0xC8] >> 4 | r[0xCF] & 0x10
}

pub(crate) fn underline_line(row_lines: usize) -> u8 {
    (row_lines - 2) as u8
}

pub(super) fn rom_bank(p1: u8) -> u8 {
    (p1 >> 7) & 1 | (p1 >> 5) & 2 | (p1 >> 3) & 4
}
