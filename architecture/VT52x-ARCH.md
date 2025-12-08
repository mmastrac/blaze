# VT52x Architecture

## Bank Switching

P1.4/P1.5/P1.6 are used to select the ROM bank.

510: P1.0 -> some sort of status flag

## Registers

6832: Video base page (read twice?)


7FC9:
 - Swaps out low ROM for some read-only register bank (510) - might be the UART channels?
 - Also seems to control memory paging at 8000?

7FCF:
 - `x... ....` - 60/70hz control

7FF8: PS2 keyboard write byte

7FF9: PS2 Clock? (0x40)

7FFA: PS2 keyboard read byte

7FFB: PS2 status (write 0xe seems to be a "next" cmd)
 - `.x.. ....` - "keyboard clock"?
 - `..x. ....` - "ready" bit?
 - `...x ....` - retrace?
 - `.... xx..` - errors?
 - `.... ..x.` - "read ready"?

// These might be a GPIO set/reset?
7FFC:
 - Written for errors, one nibble at a time

7FFE:
 - Written for errors, one nibble at a time
