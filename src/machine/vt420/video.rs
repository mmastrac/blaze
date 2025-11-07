//! Video timing constants for the VT420 terminal. These are based on some
//! experiments where we ensure that the function that waits for the composite
//! sync signal passes correctly, and the self-test for number of csync pulses
//! per frame returns both the correct timing and correct number of pulses.

use crate::machine::generic::vsync::Timing;

/// The number of vertical lines expected by the ROM
pub const VERTICAL_LINES: usize = 417;

pub const TIMING_60HZ: Timing = Timing {
    h_active: 20,
    h_fp: 2,
    h_sync: 6,
    h_bp: 4,                         // Htot = 32
    v_active: VERTICAL_LINES as u16, // Expected by ROM
    v_fp: 4,
    v_sync: 16,
    v_bp: 188, // Vtot = 625
};

pub const TIMING_70HZ: Timing = Timing {
    h_active: 20,
    h_fp: 2,
    h_sync: 6,
    h_bp: 4,                         // Htot = 32
    v_active: VERTICAL_LINES as u16, // Expected by ROM
    v_fp: 3,
    v_sync: 16,
    v_bp: 100, // Vtot = 536
};

pub struct Mapper {
    pub mapper: [u8; 16],
    pub mapper2: [u8; 16], // 6, 9, a, b, c can be written twice
}

impl Mapper {
    pub fn new() -> Self {
        let mut new = Self {
            mapper: [0; 16],
            mapper2: [0; 16],
        };
        new.mapper[3] = 0xff;
        new.mapper[4] = 0xff;
        new.mapper[5] = 0xf4;
        new
    }

    pub fn set(&mut self, offset: u8, value: u8) {
        self.mapper2[offset as usize] = self.mapper[offset as usize];
        self.mapper[offset as usize] = value;
    }

    pub fn get(&self, offset: u8) -> u8 {
        self.mapper[offset as usize]
    }

    pub fn get2(&self, offset: u8) -> u8 {
        self.mapper2[offset as usize]
    }

    pub fn sram_mapped(&self) -> u32 {
        self.sram_mapped_value(self.mapper[3])
    }

    pub fn sram_mapped_value(&self, value: u8) -> u32 {
        (value & 0x20 != 0) as u32
    }

    pub fn vram_page(&self) -> u32 {
        self.sram_mapped_value(self.mapper[5])
    }

    pub fn vram_page_value(&self, value: u8) -> u32 {
        (value & 0x08 != 0) as u32
    }

    pub fn row_count(&self, vram: &[u8]) -> Option<u8> {
        let r1 = self.get(6);
        let r2 = self.get2(6);

        // Vertical refresh
        if r1 & 0xf0 == 0xf0 || r2 & 0xf0 == 0xf0 {
            return None;
        }

        // if r1 == r2 {
        //     return Some(match r1 {
        //         0xd0 => 26,
        //         0x9a => 38,
        //         0x78 => 50,
        //         _ => 26,
        //     });
        // }

        let rh1 = ((r1 & 0x0f) + 15) % 16 + 1;
        let rh2 = ((r2 & 0x0f) + 15) % 16 + 1;

        debug_assert!(rh1 > 0 && rh1 <= 16);
        debug_assert!(rh2 > 0 && rh2 <= 16);

        // Look for the row with 0x02 set as the splitter row
        // Accumulate lines until we hit 414

        let mut remaining = VERTICAL_LINES;
        let mut screen = 0;
        let mut count = 0;
        for i in 0..50 * 2 {
            let row_attrs = vram[i * 2 + 1];
            if row_attrs & 0x02 != 0 {
                screen = 1 - screen;
            }
            let rh = if screen == 0 { rh1 } else { rh2 };
            if rh as usize > remaining {
                return Some(count as u8);
            }
            remaining -= rh as usize;
            count += 1;
        }
        Some(count)
    }

    pub fn is_blink(&self) -> bool {
        self.get(3) & 0x40 != 0
    }
}

/// Decode the VRAM into a grid of characters and attributes.
/// The row_callback is called for each row, with the row index and the row attributes.
/// The column_callback is called for each column, with the column, display character and its attributes.
#[inline(always)]
pub fn decode_vram<T>(
    vram: &[u8],
    mapper: &Mapper,
    mut row_callback: impl FnMut(&mut T, u8, u8),
    mut column_callback: impl FnMut(&mut T, u8, char, u16),
    mut data: T,
) -> T {
    let vram_base = 0;

    let Some(rows) = mapper.row_count(vram) else {
        return data;
    };

    let mut line = [0_u16; 256];
    let mut attr = [0_u8; 256];

    for row_idx in 0..rows as u16 {
        let row_header_offset = vram_base + row_idx as usize * 2;
        let row_pointer = ((vram[row_header_offset] as u16) >> 1) << 8;
        if row_pointer == 0 {
            continue;
        }

        let row_attrs = vram[row_header_offset + 1];
        let is_double_width = row_attrs & (1 << 2) != 0;
        let row_is_132 = vram[row_header_offset] & 1 != 0;

        row_callback(&mut data, row_idx as u8, row_attrs);

        line.fill(0);
        attr.fill(0);

        // Decode 12-bit character codes from packed 3-byte sequences
        let mut b = 0_u16;
        let mut j = 0_usize;
        let row_addr = row_pointer as usize;

        // First segment: 72 chars, bytes 0-107
        for i in 0..108 {
            let char_byte = vram[row_addr + i];
            match i % 3 {
                0 => b = char_byte as u16,
                1 => {
                    b |= ((char_byte & 0xf) as u16) << 8;
                    line[j] = b;
                    j += 1;
                    b = ((char_byte & 0xf0) as u16) >> 4;
                }
                _ => {
                    b |= (char_byte as u16) << 4;
                    line[j] = b;
                    j += 1;
                }
            }
        }

        // Second segment: bytes 128-220
        for i in 128..221 {
            let char_byte = vram[row_addr + i];
            let i = i + 1;
            match i % 3 {
                0 => b = char_byte as u16,
                1 => {
                    b |= ((char_byte & 0xf) as u16) << 8;
                    line[j] = b;
                    j += 1;
                    b = ((char_byte & 0xf0) as u16) >> 4;
                }
                _ => {
                    b |= (char_byte as u16) << 4;
                    line[j] = b;
                    j += 1;
                }
            }
        }

        // Extract attributes
        for i in 1..133 {
            let bit = ((i % 4) * 2) as u8;
            attr[i - 1] = (vram[row_addr + 0xdd + (i / 4)] >> bit) & 0x3;
            let cell_attr = ((line[i - 1] & 0xf00) >> 8) as u8;
            attr[i - 1] |= cell_attr << 2;
        }

        let max_columns = if row_is_132 { 132 } else { 80 };
        let decoded_columns = max_columns.min(j);

        for col in 0..decoded_columns {
            let value = line[col];
            let char_code = (value & 0xff) as u8;
            let ch = if value & 0x100 != 0 {
                match char_code {
                    0x9c => 'S',
                    0x0d => 'H',
                    0x54 => 'e',
                    0x09 => 's',
                    0x52 => 'd',
                    0x55 => 'i',
                    0x6d => 'l',
                    0x7f => 'o',
                    0x75 => 'n',
                    0x20 => '1',
                    0x38 => '2',
                    _ => '.',
                }
            } else if char_code == 0 || char_code == 0x98 {
                ' '
            } else if char_code < 0x20 || char_code > 0x7e {
                match char_code {
                    0x0d => '╭',
                    0x0c => '╮',
                    0x0e => '╰',
                    0x0b => '╯',
                    0x12 => '─',
                    0x19 => '│',
                    0xa9 => '©',
                    _ => '.',
                }
            } else {
                char::from(char_code)
            };

            let mut combined_attr = (value & 0xf00) as u16 | attr[col] as u16;
            if is_double_width {
                combined_attr |= 1 << 12;
            }
            if row_is_132 {
                combined_attr |= 1 << 13;
            }

            if char_code == 0 && (attr[col] >> 2) == 0xe {
                column_callback(&mut data, col as u8, ' ', combined_attr);
            } else {
                column_callback(&mut data, col as u8, ch, combined_attr);
            }
        }
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::generic::vsync::SyncGen;

    #[test]
    fn test_sync_gen_60hz() {
        let mut sync_gen = SyncGen::new(TIMING_60HZ);
        let mut csync_low = false;
        let mut line_count = 0;
        for _ in 0..TIMING_60HZ.pixel_tot() {
            let next = sync_gen.tick();
            if csync_low && !next {
                line_count += 1;
            }
            csync_low = next;
        }

        assert_eq!(sync_gen.x, 0);
        assert_eq!(sync_gen.y, 0);

        assert_eq!(line_count, TIMING_60HZ.vtot());
        assert!((0x4e00..0x4f00).contains(&TIMING_60HZ.pixel_tot()));
    }

    #[test]
    fn test_sync_gen_70hz() {
        let mut sync_gen = SyncGen::new(TIMING_70HZ);
        let mut csync_low = false;
        let mut line_count = 0;
        for _ in 0..TIMING_70HZ.pixel_tot() {
            let next = sync_gen.tick();
            if csync_low && !next {
                line_count += 1;
            }
            csync_low = next;
        }

        assert_eq!(sync_gen.x, 0);
        assert_eq!(sync_gen.y, 0);

        assert_eq!(line_count, TIMING_70HZ.vtot());
        assert!((0x4300..0x4400).contains(&TIMING_70HZ.pixel_tot()));
    }

    #[test]
    fn test_32_clocks_per_line() {
        assert_eq!(TIMING_60HZ.htot(), 32);
        assert_eq!(TIMING_70HZ.htot(), 32);
    }

    #[test]
    fn test_line_count() {
        assert_eq!(TIMING_60HZ.vtot(), 625);
        assert_eq!(TIMING_70HZ.vtot(), 536);
    }

    /// Syncable means that we capture pulses of 15 off, then 15 on at some
    /// point. If this test passes, we should successfully boot the system past
    /// diagnostics.
    #[test]
    fn test_syncable() {
        for timing in [TIMING_60HZ, TIMING_70HZ] {
            let mut runs = Vec::new();
            let mut current_value = None;
            let mut current_count = 0;
            let mut sync_gen = SyncGen::new(timing);
            for _ in 0..timing.pixel_tot() / 4 {
                sync_gen.tick();
            }

            for _ in 0..timing.pixel_tot() {
                let next = sync_gen.tick();
                if Some(next) == current_value {
                    current_count += 1;
                } else {
                    if let Some(value) = current_value {
                        runs.push((value, current_count));
                    }
                    current_value = Some(next);
                    current_count = 1;
                }
            }
            runs.push((current_value.unwrap(), current_count));
            println!("Runs: {:?}", runs);
            assert!(
                runs.windows(2)
                    .any(|w| w[0].0 && w[0].1 >= 15 && !w[1].0 && w[1].1 >= 15)
            );
        }
    }
}
