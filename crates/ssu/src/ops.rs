//! SSU operation encoding and decoding

use core::fmt;
use core::ops::{self};

use tracing::trace;

/// Maximum command length in bytes
pub const MAX_COMMAND_LEN: usize = 128;

/// Maximum label length in bytes
pub const MAX_LABEL_LEN: usize = 64;

// Frame delimiters
pub const INTRO: u8 = 0x14;
pub const TERM: u8 = 0x1C;
pub const US: u8 = 0x1F; // Unit Separator

// Opcodes (all of these use INTRO/TERM)

/// Probe: !@AB
pub const OP_PROBE: u8 = 0x21; // '!' - Probe/Enable
pub const OP_OPEN: u8 = 0x22; // '"' - Open session
pub const OP_SELECT: u8 = 0x23; // '#' - Select session
pub const OP_RESET: u8 = 0x2A; // '*' - Reset
pub const OP_ADDCR: u8 = 0x2B; // '+' - Add credits
pub const OP_VERIFY: u8 = 0x2D; // '-' - Verify credits
pub const OP_CLOSE: u8 = 0x2E; // '.' - Close session
pub const OP_DISABLE: u8 = 0x2F; // '/' - Disable
pub const OP_ZERO: u8 = 0x30; // '0' - Zero credits
pub const OP_SEND_BREAK: u8 = 0x3A; // ':' - Send break
pub const OP_REQUEST_RESTORE: u8 = 0x3B; // ';' - Request restore
pub const OP_RESTORE: u8 = 0x3C; // '<' - Restore
pub const OP_REPORT: u8 = 0x3D; // '=' - Report/Ack
pub const OP_RESTORE_END: u8 = 0x3E; // '>' - Restore end
pub const OP_QUERY: u8 = 0x3F; // '?' - Query session

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum SSUOpcode {
    Probe = OP_PROBE,
    Open = OP_OPEN,
    Select = OP_SELECT,
    Reset = OP_RESET,
    AddCredits = OP_ADDCR,
    Verify = OP_VERIFY,
    Close = OP_CLOSE,
    Disable = OP_DISABLE,
    Zero = OP_ZERO,
    SendBreak = OP_SEND_BREAK,
    RequestRestore = OP_REQUEST_RESTORE,
    Restore = OP_RESTORE,
    Report = OP_REPORT,
    RestoreEnd = OP_RESTORE_END,
    Query = OP_QUERY,
}

impl TryFrom<u8> for SSUOpcode {
    type Error = ParseError;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            OP_PROBE => Ok(SSUOpcode::Probe),
            OP_OPEN => Ok(SSUOpcode::Open),
            OP_SELECT => Ok(SSUOpcode::Select),
            OP_RESET => Ok(SSUOpcode::Reset),
            OP_ADDCR => Ok(SSUOpcode::AddCredits),
            OP_VERIFY => Ok(SSUOpcode::Verify),
            OP_CLOSE => Ok(SSUOpcode::Close),
            OP_DISABLE => Ok(SSUOpcode::Disable),
            OP_ZERO => Ok(SSUOpcode::Zero),
            OP_SEND_BREAK => Ok(SSUOpcode::SendBreak),
            OP_REQUEST_RESTORE => Ok(SSUOpcode::RequestRestore),
            OP_RESTORE => Ok(SSUOpcode::Restore),
            OP_REPORT => Ok(SSUOpcode::Report),
            OP_RESTORE_END => Ok(SSUOpcode::RestoreEnd),
            OP_QUERY => Ok(SSUOpcode::Query),
            _ => Err(ParseError::UnknownOpcode(value)),
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum SSUString<const LEN: usize> {
    Embedded([u8; LEN], u8),
    External(&'static [u8]),
}

impl<const LEN: usize> Default for SSUString<LEN> {
    fn default() -> Self {
        // Cheapest to construct
        SSUString::External(&[])
    }
}

impl<const LEN: usize> fmt::Debug for SSUString<LEN> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SSUString::Embedded(s, len) => write!(
                f,
                "Embedded({:?})",
                String::from_utf8_lossy(&s[..*len as usize])
            ),
            SSUString::External(s) => write!(f, "External({:?})", String::from_utf8_lossy(s)),
        }
    }
}

impl<const LEN: usize> ops::Deref for SSUString<LEN> {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        match self {
            SSUString::Embedded(s, len) => &s[0..*len as usize],
            SSUString::External(s) => s,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum SSUState {
    Disabled = 0,
    Enabled = 1,
    EnabledWithSessions = 2,
}

/// SSU operation types
#[derive(Debug, Default, Copy, Clone, PartialEq, Eq)]
pub enum SSUOp<const OPEN_LEN: usize> {
    /// Probe: INTRO OP_PROBE state protocol_variant max_sessions TERM
    /// Parameters: state (u8), protocol_variant (u8), max_sessions (u8)
    Probe(SSUState, u8, u8),
    /// Disable: INTRO OP_DISABLE TERM
    #[default]
    Disable,
    /// Open: INTRO OP_OPEN sid US label US TERM
    /// Parameters: session_id (u8), label ([u8; MAX_LABEL_LEN]), label_len (usize)
    Open {
        session_id: u8,
        label: SSUString<OPEN_LEN>,
    },
    /// Select: INTRO OP_SELECT sid TERM
    /// Parameters: session_id (u8)
    Select(u8),
    /// Reset: INTRO OP_RESET sid TERM
    Reset(Option<u8>),
    /// Close: INTRO OP_CLOSE sid status TERM
    Close(u8, bool),
    /// Add credits: INTRO OP_ADDCR sid x y z TERM
    /// Parameters: session_id (u8), credits (usize)
    AddCredits { session_id: u8, credits: usize },
    /// Verify: INTRO OP_VERIFY sid TERM
    /// Parameters: session_id (u8)
    Verify(u8),
    /// Query: INTRO OP_QUERY sid TERM
    /// Parameters: session_id (u8)
    Query(u8),
    /// Zero: INTRO OP_ZERO sid TERM
    /// Parameters: session_id (u8)
    Zero(u8),
    /// Send a break to a given session
    Break(u8),
    /// Request restore: INTRO OP_REQUEST_RESTORE TERM
    RequestRestore,
    /// Restore: INTRO OP_RESTORE TERM
    Restore,
    /// Report/Ack: INTRO OP_REPORT op_being_acked sid code TERM
    /// Parameters: op_being_acked (u8), session_id (u8), code (u8)
    Report {
        op: SSUOpcode,
        session_id: Option<u8>,
        code: u8,
    },
    /// Restore end: INTRO OP_RESTORE_END TERM
    RestoreEnd,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// Frame doesn't start with INTRO
    MissingIntro,
    /// Frame doesn't end with TERM
    MissingTerm,
    /// Unknown opcode
    UnknownOpcode(u8),
    /// Frame too short
    TooShort,
    /// Frame too long
    TooLong,
    /// Invalid parameter format
    InvalidParameter,
}

impl<const OPEN_LEN: usize> SSUOp<OPEN_LEN> {
    /// Parse a frame from bytes
    ///
    /// The frame must start with INTRO and end with TERM
    pub fn parse(frame: &[u8]) -> Result<Self, ParseError> {
        if frame.is_empty() {
            return Err(ParseError::TooShort);
        }
        if frame[0] != INTRO {
            return Err(ParseError::MissingIntro);
        }
        if frame.len() < 3 {
            return Err(ParseError::TooShort);
        }
        if frame[frame.len() - 1] != TERM {
            return Err(ParseError::MissingTerm);
        }
        if frame.len() > MAX_COMMAND_LEN {
            return Err(ParseError::TooLong);
        }

        trace!("Parsing frame: {:?}", String::from_utf8_lossy(frame));

        let opcode = frame[1];
        let params = &frame[2..frame.len() - 1];

        match opcode {
            OP_PROBE => {
                if params.len() != 3 {
                    return Err(ParseError::InvalidParameter);
                }
                Ok(SSUOp::Probe(
                    match params[0].wrapping_sub(b'@') {
                        0 => SSUState::Disabled,
                        1 => SSUState::Enabled,
                        2 => SSUState::EnabledWithSessions,
                        _ => return Err(ParseError::InvalidParameter),
                    },
                    params[1].wrapping_sub(b'@'),
                    params[2].wrapping_sub(b'@'),
                ))
            }
            OP_OPEN => {
                // Format: session_id US label US TERM
                // Even if label is empty, format is: session_id US US TERM
                if params.is_empty() {
                    return Err(ParseError::InvalidParameter);
                }
                let session_id = params[0].wrapping_sub(b'A');

                // Must have at least session_id US US (empty label)
                if params.len() < 3 || params[1] != US {
                    return Err(ParseError::InvalidParameter);
                }

                // Find the second US separator
                let label_start = 2;
                let mut label_end = params.len();
                let mut found_second_us = false;

                for (i, &b) in params.iter().enumerate().skip(2) {
                    if b == US {
                        label_end = i;
                        found_second_us = true;
                        break;
                    }
                }

                if !found_second_us {
                    return Err(ParseError::InvalidParameter);
                }

                let label_slice = &params[label_start..label_end];
                if label_slice.len() > OPEN_LEN {
                    return Err(ParseError::TooLong);
                }

                let mut label = [0u8; OPEN_LEN];
                label[..label_slice.len()].copy_from_slice(label_slice);
                let label_len = label_slice.len();

                Ok(SSUOp::Open {
                    session_id,
                    label: SSUString::Embedded(label, label_len as u8),
                })
            }
            OP_CLOSE => {
                if params.len() != 2 {
                    return Err(ParseError::InvalidParameter);
                }
                Ok(SSUOp::Close(
                    params[0].wrapping_sub(b'A'),
                    params[1] == b'@',
                ))
            }
            OP_SELECT => {
                if params.is_empty() {
                    return Err(ParseError::InvalidParameter);
                }
                Ok(SSUOp::Select(params[0].wrapping_sub(b'A')))
            }
            OP_RESET => {
                if params.len() != 1 {
                    return Err(ParseError::InvalidParameter);
                }
                if params[0] == b'@' {
                    Ok(SSUOp::Reset(None)) // all
                } else {
                    Ok(SSUOp::Reset(Some(params[0].wrapping_sub(b'A'))))
                }
            }
            OP_ADDCR => {
                let session_id = params[0].wrapping_sub(b'A');
                let (x, y, mut z) = match params.len() {
                    1 => (0, 0, 0),
                    2 => (0, 0, params[1].wrapping_sub(b'@')),
                    3 => (
                        0,
                        params[1].wrapping_sub(b' '),
                        params[2].wrapping_sub(b'@'),
                    ),
                    4 => (
                        params[1].wrapping_sub(b' '),
                        params[2].wrapping_sub(b' '),
                        params[3].wrapping_sub(b'@'),
                    ),
                    _ => {
                        return Err(ParseError::InvalidParameter);
                    }
                };
                let z5 = z & 0x20 != 0;
                z &= 0x1F;
                let credits = ((x as u16 & 0x1F) << 10)
                    | ((y as u16 & 0x1F) << 5)
                    | (z as u16 & 0x1F)
                    | ((z5 as u16) << 11);
                Ok(SSUOp::AddCredits {
                    session_id,
                    credits: credits as usize,
                })
            }
            OP_VERIFY => {
                if params.is_empty() {
                    return Err(ParseError::InvalidParameter);
                }
                Ok(SSUOp::Verify(params[0].wrapping_sub(b'A')))
            }
            OP_DISABLE => {
                if params.len() != 3 {
                    return Err(ParseError::InvalidParameter);
                }
                // Unsure if these have meaning
                if params[0] != b'@' || params[1] != b'@' || params[2] != b'@' {
                    return Err(ParseError::InvalidParameter);
                }
                Ok(SSUOp::Disable)
            }
            OP_ZERO => {
                if params.is_empty() {
                    return Err(ParseError::InvalidParameter);
                }
                Ok(SSUOp::Zero(params[0].wrapping_sub(b'A')))
            }
            OP_REQUEST_RESTORE => {
                if !params.is_empty() {
                    return Err(ParseError::InvalidParameter);
                }
                Ok(SSUOp::RequestRestore)
            }
            OP_RESTORE => {
                if !params.is_empty() {
                    return Err(ParseError::InvalidParameter);
                }
                Ok(SSUOp::Restore)
            }
            OP_REPORT => {
                if params.len() < 3 {
                    return Err(ParseError::InvalidParameter);
                }
                let op = params[0].try_into()?;
                let session_id = if params[1] == b'a' {
                    None
                } else {
                    Some(params[1].wrapping_sub(b'A'))
                };
                let code = params[2].wrapping_sub(b'@');
                Ok(SSUOp::Report {
                    op,
                    session_id,
                    code,
                })
            }
            OP_RESTORE_END => {
                if !params.is_empty() {
                    return Err(ParseError::InvalidParameter);
                }
                Ok(SSUOp::RestoreEnd)
            }
            OP_SEND_BREAK => {
                if params.is_empty() {
                    return Err(ParseError::InvalidParameter);
                }
                Ok(SSUOp::Break(params[0].wrapping_sub(b'A')))
            }
            OP_QUERY => {
                if params.is_empty() {
                    return Err(ParseError::InvalidParameter);
                }
                Ok(SSUOp::Query(params[0].wrapping_sub(b'A')))
            }
            _ => Err(ParseError::UnknownOpcode(opcode)),
        }
    }

    /// Serialize the operation to a buffer
    ///
    /// Returns a slice of the buffer containing the serialized frame
    pub fn serialize<'b>(
        &self,
        buf: &'b mut [u8; MAX_COMMAND_LEN],
    ) -> Result<&'b [u8], ParseError> {
        let mut pos = 0;

        buf[pos] = INTRO;
        pos += 1;

        match self {
            SSUOp::Probe(state, protocol_variant, max_sessions) => {
                buf[pos] = OP_PROBE;
                pos += 1;
                buf[pos] = (*state as u8) + b'@';
                pos += 1;
                buf[pos] = *protocol_variant + b'@';
                pos += 1;
                buf[pos] = *max_sessions + b'@';
                pos += 1;
            }
            SSUOp::Open { session_id, label } => {
                buf[pos] = OP_OPEN;
                pos += 1;
                buf[pos] = *session_id + b'A';
                pos += 1;
                // Always include US separators, even if label is empty
                buf[pos] = US;
                pos += 1;
                let label_len = label.len();
                if pos + label_len + 1 > MAX_COMMAND_LEN {
                    return Err(ParseError::TooLong);
                }
                if label_len > 0 {
                    buf[pos..pos + label_len].copy_from_slice(&label[..label_len]);
                    pos += label_len;
                }
                buf[pos] = US;
                pos += 1;
            }
            SSUOp::Close(session_id, ok) => {
                buf[pos] = OP_CLOSE;
                pos += 1;
                buf[pos] = *session_id + b'A';
                pos += 1;
                buf[pos] = if *ok { b'@' } else { b'e' };
                pos += 1;
            }
            SSUOp::Select(session_id) => {
                buf[pos] = OP_SELECT;
                pos += 1;
                buf[pos] = *session_id + b'A';
                pos += 1;
            }
            SSUOp::Reset(session_id) => {
                buf[pos] = OP_RESET;
                pos += 1;
                if let Some(session_id) = session_id {
                    buf[pos] = *session_id + b'A';
                } else {
                    buf[pos] = b'@';
                }
                pos += 1;
            }
            SSUOp::AddCredits {
                session_id,
                credits,
            } => {
                buf[pos] = OP_ADDCR;
                pos += 1;
                buf[pos] = *session_id + b'A';
                pos += 1;

                // Credits = { z5, x4, x3, x2, x1, x0, y4, y3, y2, y1, y0, z4, z3, z2, z1, z0 }
                let total = *credits as u16;
                let x = ((total >> 10) & 0x1F) as u8;
                let y = ((total >> 5) & 0x1F) as u8;
                let mut z = (total & 0x1F) as u8;

                if (total & 0x8000) != 0 {
                    z |= 0x20; // set z5 (bit5 of z byte)
                }

                if x > 0 {
                    buf[pos] = x + b' ';
                    pos += 1;
                }
                if x > 0 || y > 0 {
                    buf[pos] = y + b' ';
                    pos += 1;
                }
                if x > 0 || y > 0 || z > 0 {
                    buf[pos] = z + b'@';
                    pos += 1;
                }
            }
            SSUOp::Verify(session_id) => {
                buf[pos] = OP_VERIFY;
                pos += 1;
                buf[pos] = *session_id + b'A';
                pos += 1;
            }
            SSUOp::Disable => {
                buf[pos] = OP_DISABLE;
                pos += 1;
                for _ in 0..3 {
                    buf[pos] = b'@';
                    pos += 1;
                }
            }
            SSUOp::Zero(session_id) => {
                buf[pos] = OP_ZERO;
                pos += 1;
                buf[pos] = *session_id + b'A';
                pos += 1;
            }
            SSUOp::RequestRestore => {
                buf[pos] = OP_REQUEST_RESTORE;
                pos += 1;
            }
            SSUOp::Restore => {
                buf[pos] = OP_RESTORE;
                pos += 1;
            }
            SSUOp::Report {
                op,
                session_id,
                code,
            } => {
                buf[pos] = OP_REPORT;
                pos += 1;
                buf[pos] = *op as _;
                pos += 1;
                buf[pos] = if let Some(session_id) = session_id {
                    session_id + b'A'
                } else {
                    b'a'
                };
                pos += 1;
                buf[pos] = code + b'@';
                pos += 1;
            }
            SSUOp::RestoreEnd => {
                buf[pos] = OP_RESTORE_END;
                pos += 1;
            }
            SSUOp::Break(session_id) => {
                buf[pos] = OP_SEND_BREAK;
                pos += 1;
                buf[pos] = *session_id + b'A';
                pos += 1;
            }
            SSUOp::Query(session_id) => {
                buf[pos] = OP_QUERY;
                pos += 1;
                buf[pos] = *session_id + b'A';
                pos += 1;
            }
        }

        buf[pos] = TERM;
        pos += 1;

        Ok(&buf[..pos])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_add_credits_message() {
        // This is the binary representation of: INTRO, '+', 'B', '"', '@', TERM
        let msg: &[u8] = &[0x14, b'+', b'B', b'"', b'@', 0x1c];
        let op = SSUOp::<MAX_LABEL_LEN>::parse(msg);
        assert!(op.is_ok(), "Failed to parse AddCR message: {op:?}");
        if let Ok(SSUOp::AddCredits {
            session_id,
            credits,
        }) = op
        {
            assert_eq!(session_id, 1);
            assert_eq!(credits, 64);
        } else {
            panic!("Did not parse as AddCR: {op:?}");
        }
    }

    #[test]
    fn test_serialize_add_credits_message() {
        let mut buf = [0u8; MAX_COMMAND_LEN];
        let msg = SSUOp::<MAX_LABEL_LEN>::AddCredits {
            session_id: 1,
            credits: 64,
        }
        .serialize(&mut buf)
        .unwrap();
        assert_eq!(msg, &[0x14, b'+', b'B', b'"', b'@', 0x1c]);
    }
}
