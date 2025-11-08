#![doc = include_str!("SSU.md")]

// Frame delimiters
const INTRO: u8 = 0x14;
const TERM: u8 = 0x1C;
const US: u8 = 0x1F; // Unit Separator

// Opcodes (all of these use INTRO/TERM)

/// Probe: !@AB
const OP_PROBE: u8 = 0x21; // '!' - Probe/Enable
const OP_OPEN: u8 = 0x22; // '"' - Open session
const OP_SELECT: u8 = 0x23; // '#' - Select session
const OP_RESET: u8 = 0x2A; // '*' - Reset
const OP_ADDCR: u8 = 0x2B; // '+' - Add credits
const OP_VERIFY: u8 = 0x2D; // '-' - Verify credits
const OP_DISABLE: u8 = 0x2F; // '/' - Disable
const OP_ZERO: u8 = 0x30; // '0' - Zero credits
const OP_REQUEST_RESTORE: u8 = 0x3B; // ';' - Request restore
const OP_RESTORE: u8 = 0x3C; // '<' - Restore
const OP_REPORT: u8 = 0x3D; // '=' - Report/Ack
const OP_RESTORE_END: u8 = 0x3E; // '>' - Restore end
