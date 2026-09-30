# VT510 Architecture

Firmware 23-032ED-00 (`roms/vt510/23-032ED-00.bin`, 512KB). The code is the
same family as the VT520/VT525 (see VT52x-ARCH.md), on an older gate array.
The i8051 database is `roms/vt510/vt510.dsl` (generated) plus
`vt510.diff.dsl`.

## Bank Switching

Eight 64KB banks, in file order. The bank bits sit on different pins from the
VT520:

| Address line | VT510 | VT520/525 |
|--------------|-------|-----------|
| A16 | P1.7 | P1.4 |
| A17 | P1.6 | P1.5 |
| A18 | P1.5 | P1.6 |

All 1038 cross-bank stubs (0x0E70-0x1CE4 in each bank) land on a matching
slot with this order, against 582 with P1.6 and P1.7 swapped. The stubs work
as on the VT520: bit ops, `LCALL` to their own address, restore, `RET`.

The ROM ID "DEC510" is at 0x2A, after an `AA 55` marker at 0x26.

## Reset

1. Banks 0-3 jump to 0x0E70, which sets P1.5 (bank 4). Banks 4-7 reset to
   their own entry (bank 4 0x1140, 5 0x13A4, 6 0x1434, 7 0x17F0).
2. Bank 4's 0x1140 sets P1.6 and P1.7 and lands in bank 7 at `reset_entry`
   (0x17F0), which matches the VT525's bank 7 0x2947.
3. `reset_entry` loads the power-up register table (0x1921), tests internal
   RAM, checks for the factory loopback on comm1 (B = 0xB6), runs the ROM and
   RAM tests and `selftest_dispatch` (0x176D).
4. It leaves through 0x0F08 to bank 0 `main_entry` (0x1CEC). `boot_wait_loop`
   (0x1D1F) ends on Set-Up, a key or host data, as on the VT520.

The power-up table holds (register, value) pairs (the VT520's has three bytes
per entry): F6=00 D2=40 D3=11 CF=01 CF=04 D0=04 C8=FE C9=03 CA=0F CB=3F
CC=3F E2=C0 ED=C0 EB=01 EC=55 EF=FB F0=00 F2=64 F3=FC F1=04 D9=02 ED=C0
ED=C0 F4=80. Comm1 matches the VT520 except the receive ring page (0x64).
There are no comm2 entries.

## Registers

Differences from the VT520:

- No frame-synced hardware copies. The VT520 copies staging registers to
  live ones at the frame; the VT510's frame interrupt copies RAM shadows
  instead (below).
- 0x7FC9 selects the RAM page behind 0x8000-0xFFFF. The RAM test writes 0-3,
  so 128KB. (VT520: 0x7FB6; its 0x7FC9 is the blitter command.)
- The live registers are 0x7FC0-0x7FFF and their staging copies sit 0x40
  lower (0x7F80-0x7FBF); the VT520's staging copies are 0x80 lower. The
  firmware usually writes both (the blitter, and the comm registers through
  `ANL DPL,#0xBF` / `ORL DPL,#0x40`), sometimes only the live one. Most live
  registers are where the VT520 has them. The exceptions found so far are
  the blit command (0x7FD5, VT520 0x7FC9) and the RAM page (0x7FC9).

### Blitter

| Live (staging) | Use | VT520 |
|----------------|-----|-------|
| 0x7FC0-0x7FC2 (0x80-0x82) | Source (character for glyph blits in 0xC0, attribute in 0xC1) | same |
| 0x7FC3-0x7FC5 (0x83-0x85) | Destination (column in 0xC3, scan line in 0xC4) | same |
| 0x7FC6-0x7FC8 (0x86-0x88) | Negated counts | same |
| 0x7FCD (0x8D) | Attribute for the whole blit (Set-Up highlight 0x20) | same |
| 0x7FCE (0x8E) | | same |
| 0x7FD5 (0x95) | Command | 0x7FC9 |

P1.0 is the blitter's idle line. After some commands (the fill at bank 7
0x2376) the firmware waits for P1.0 to drop before waiting for it to rise,
so it goes low while a blit runs.

Commands: 0x98 glyph, 0xB8 soft glyph, 0xF8, 0x9A fill, 0x03 and 0x13 copy,
0x8D/0x9D. These are the VT520 commands with bit 2 clear (VT520 0x9C, 0xBC,
0xFC, 0x9E): the destination field (bits 3-2) is 10 here and 11 on the VT520.

`draw_glyph` (0x0903) takes the scan line from the firmware's row map at
XDATA 0x0100 + row (see Memory).

### Video

The frame interrupt (INT0 with 0x7FFB bit 4, bank 0 0x0165) waits for P3.4
low, then copies three RAM shadows:

| Shadow | Register | Use |
|--------|----------|-----|
| 0x76EB | 0x7FCF | Display control. Bit 7 = 60 Hz (547 lines per frame; clear = 70 Hz, 456 lines; from the self-test at bank 7 0x1A68). Bit 4 = 132 columns. Bit 6 = attribute bit 7 is dim (WYSE). Bits 2, 3 and 5 are set and cleared by bank 0 0xD583-0xD5B3; not identified. |
| 0x76EC | 0x7FD2 | Horizontal position: (setting 0x690A + 14) / 11 at 80 columns, or (0x690A + 50) / 8 at 132. Quotient in the high nibble, remainder in the low. |
| 0x76ED | 0x7FD3 | Vertical position: setting 0x6909 - 13 (- 10 with 0x6902 bit 4), clamped to 0-0x1F. |

The positions are set from Set-Up with modifier + arrow keys (bank 5
0x7E3A; bits 0x08 and 0x0A, which are Shift and Ctrl on the VT520). Shift +
Up/Down moves the picture vertically (0x6909), Shift + Left/Right
horizontally (0x690A). Ctrl + Left/Right, only when P1.3 is low,
steps 0x6908 through 16 levels; `write_level_and_modem_outputs` (0x06F6)
puts its high nibble in 0x7FF4 bits 7-4, beside the comm1 modem outputs in
bits 2-0. This is probably monitor brightness or contrast. The VT520 has none
of these.

0x7FD0 is written 0xE4, then 0x04 after a delay, on a cold start.

P3.4 (T0) is a scan-line clock, as on the VT420 (where it is composite
sync). The self-test (bank 7 0x19D7) samples it: high for 17-19 checks, then
low for 1-2. It then counts T0 pulses per frame with 0x7FCF = 0x84 and 0x04
and expects 0x0223 (547) and 0x01C8 (456).

The VT510 sits between the VT420 and the VT520: VT420-style frame timing,
60/70 Hz choice and software-copied video registers, with the VT520's
blitter and bank scheme.

## Memory

| XDATA | Use |
|-------|-----|
| 0x0000-0x001A, 0x0080-0x009A | Display line tables, one entry per display row (24 text rows, then 0xC8, 0xD0, 0xD8): the stored row's scan line / 2. Flags would be at +0x40 as on the VT520 (0x7E00/0x7E80); all zero so far. The firmware keeps both copies alike; what selects one is not known. |
| 0x0100-0x019A | The firmware's own row map: text row -> stored row (0xFF = unused), status rows at 0x0196-0x0197. Filled from the free-row pool at 0x6000 (bank 0 0x6190). |
| 0x019B- | Session text: 132 cells (0x108 bytes) per row, from the row pointers at 0x541D. The first session's 75 rows run to about 0x4E00. |
| 0x6000- | Free stored rows, NVR image at 0x6A00, other variables up to 0x7EFF. |
| 0x8000-0xFFFF | 32KB DRAM page selected by 0x7FC9 (0-3, 128KB). |

Low XDATA has no window registers (the VT520 maps 0x0000-0x17FF through
0x7FBB/0x7FBF): it is plain RAM.

The fonts are the VT520's: the same packed glyphs (about 90% of the VT520's
packed font bytes appear unchanged in bank 6, some 0x900-0xA00 lower) and the
same unpacked layout, 10x16 cells stored as 8 bits plus 2 extra bits per
line. The loader (bank 6 0x61525) takes the glyph height from XDATA 0x7689,
picks the ROM font through the table at 0x16EB, pads (17 - height) / 2
blank lines at the top, and unpacks through the window at 0xC000 with page 3
(bank 6 0x615B6-0x61615): pixel bytes from column 0, extra bits as nibbles
from column 0x50 (the low nibble for an even column, the high for an odd).

DRAM page 3 from 0x4000 then holds 640 glyphs in columns 0x00-0x4F, eight
per column (block c & 7, 0x800 each, 0x80-byte line stride): character c of
font f, line l at 0x4000 + (c & 7) * 0x800 + l * 0x80 + f * 0x20 + (c >> 3),
bit 0 leftmost, with its extra 2 bits at column 0x50 + k / 2 of the same
line (k = f * 0x20 + (c >> 3)). Fonts 0 and 1 hold 256 glyphs and font 2
128. Font 0 is the main font; Set-Up draws its boxes with font 1. There is
no font 3: its columns would be the extra bits. An earlier reading of 8-pixel
glyphs with pixel 8 repeating bit 7 missed the extra bits; they hold the
right-hand pixel of each stroke, so the letters' strokes are 2 pixels wide
on both sides.

Two small fonts follow, in columns 0x78-0x7B with their extra bits in
0x7C-0x7D (not at 0x50 + k / 2):

- Page 3 from 0x4000: 32 double-line and mixed box-drawing pieces (the PC
  code page 437 set), unpacked from ROM 0x63F0B-0x64C67.
- DRAM block 0 (and a copy in block 1): 40 glyphs from ROM 0x64C66-0x64F03,
  small digits and block pieces. Set-Up's menu arrows are soft glyph blits
  from block 1, columns 0x78-0x79 (0x7FC2 = 0x01). What makes the block 1
  copy is not known.

Glyph blits give the stored row as scan line / 2 in 0x7FC4 (0x08 per
16-line row). 0x7FC8 bits 7-4 hold the row height - 1 (0xFE at power-up).

## Keyboard

The PS/2 registers match the VT520's (0x7FF8 transmit, 0x7FF9 bit 6 line
control, 0x7FFA receive, 0x7FFB status/command). The power-up sequence (bank
7 0x1AF1) differs in its first step: the VT520 sends commands, but the VT510
releases the line and waits for the keyboard's own self-test byte (0xAA). It
watches 0x7FFB bit 5, the keyboard clock, by polling (0x1B9B): the clock
must be high, fall, then give ten more low/high cycles with each half 2 to
about 20 polls long. Then it inhibits and releases the line and reads the
byte. Each 0x0E write to 0x7FFB in that loop also acknowledges (bit 1), which
latches the byte into 0x7FFA.

If the byte is not 0xAA it sends 0xFE, then 0xFF (reset, expects 0xFA
0xAA). Then 0xF2 (identify: 0xFA 0xAB, ID) and 0xAF (0xFE from a PC
keyboard sets bit 0x12), as on the VT520. After the boot wait loop it sends
0xF5, 0xAF, 0xF0 0x03, 0xF8, 0xF4 (scan code set 3, make/break, enable).

Set-Up opens with F3 on an LK keyboard (the emulator's default) and with
Caps Lock + Print Screen on a PC keyboard.

## Comm

The channel registers match the VT520's live copies (comm1 0x7FEB-0x7FF5,
comm3 0x7FE0-0x7FEA), with staging copies 0x40 lower. There are no comm2
entries in the power-up table.

0x7FDB (comm1) and 0x7FDA (comm3), the VT520's modem status inputs, report
the transmitter here: bit 3 = it can take a byte, bit 2 = everything is
sent; both read clear while the channel is disabled (+2 bit 0). The
transmitter has a holding register and a shift register. The self-test
(bank 7 0x286D, run for comm1 by 0x2736 and comm3 by 0x2742) checks this
against the transmit interrupt cause (+5 bit 3) and INT1 (P3.3):

1. Disabled: bit 3 clear.
2. +5 = 0x08, +2 = 0x01: bits 3 and 2 set, +5 bit 3 set, INT1 low.
3. Two bytes written: bits 3 and 2 clear, +5 bit 3 clear, INT1 high.
4. After one byte time (0x2DE5, from a table by speed): bit 3 set, bit 2
   clear, INT1 low; after another, bit 2 set.

A failure sets IDATA 0x1F bit 1 ("RS-232 Port Data Error - 2") or bit 4
("DEC-423 Port Error - 5"). Later steps (0x2938 onward) send patterns at
each speed and read them back.

## NVR

The same 24C16-style EEPROM on P1.1 (SCL) and P1.2 (SDA), device 0xA0.
Its image lives in RAM at 0x6A00 (bank 0 0xE423: EEPROM address = DPTR -
0x6A00). 0xE348 reads a block, 0xE315 reads and checks, and 0xE377 compares
each byte with RAM and writes only the ones that differ (0xE3E1 writes one
byte, polls, reads it back). The START routine (0xE4A7) checks SDA and SCL
at every step. `roms/vt510/nvr-factory.bin` was saved from Set-Up ("Save
settings") after a boot with a blank NVR.

## Emulator

`--machine vt510` runs the VT52x model with the VT510 ROM (with
`--machine vt52x`, pass the VT510 ROM). The model detects it ("DEC510") and
switches:
bank bits, the register map above (staging to live, 0xD5 to the blit
command, 0xC9 to the RAM page), glyph and fill commands with bit 2 set, a
20-instruction scan line with T0 high for 18, 547 or 456 lines per frame
from 0x7FCF bit 7, and 8 instructions of P1.0 busy after each command. The
keyboard queues 0xAA at power-up, and 0x7FFB bit 5 toggles 11 times (15
instructions per half, after 100 instructions idle) for each byte the
keyboard sends while the line is released. The comm transmitters have a
holding and a shift register, with a byte time from the transmit speed, and
report them in 0x7FDB/0x7FDA (see Comm). The NVR starts from
`roms/vt510/nvr-factory.bin`, saved with "On-line" checked.

With that the VT510 passes all its power-up tests, shows host text, and
sends typed keys to the host.

Low XDATA is plain RAM in the model (an earlier version mapped
0x1000-0x17FF through the VT520's window B, so clearing session text at
0x1113 wiped the line tables). The display reads the line table at 0x0000,
takes the row height from 0x7FC8 and 132 columns from 0x7FCF bit 4, and
draws glyphs from the fonts above. The underline line is not known; the
model uses 2 lines above the row bottom.

## Not Yet Known

- The line table select, flags, and the underline line.
- Which attributes pick fonts 1 and 2, and what uses font 2 and the small
  fonts' digits.
- 0x7FCF bits 2, 3 and 5; 0x7FC8; 0x7FD5 reads in banks 4 and 5.
- What P1.3 senses (read about 26 times; the VT520 only sets it).
