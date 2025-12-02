# DEC SSU (Session Setup Utility) Protocol

a.k.a. DEC TD/SMP: Terminal Device/Session Management Protocol
(<https://en.wikipedia.org/wiki/TD/SMP>).

## What is it?

SSU is a protocol used to setup and manage multiple sessions on a terminal
device, multiplexed over a single physical connection.

## Protocol

The protocol is somewhat described in patent US5165020 ("Terminal Device/Session
Management Protocol") from 1991, but the details are omitted and were
painstakingly reverse engineered from a real VT420 terminal.

### Flow Control

SSU uses a simple flow control mechanism based on credits. Each side is
allocated a number of credits to use for sending data, and the strategy for
dispensing them is left to the implementation. Generally, the credits will
reflect the available buffer space for the session.

By default, each side of the session has no credits and must be granted credits
before sending data.

When a side runs out of credits on a given channel, its credits have been
explicitly zeroed/reset, or if it has never been granted credits, it must not
send any more data until it explicitly receives more credits.

Each side should preemptively add more credits as it detects the peer is running
low. If the remote side continues sending data after running out of credits, the
local side can send a `ZERO` message to force it to zero out its credit balance
until there's enough buffer space to receive data from it again.

The entire balance of credit does not need to be dispensed at once. The sender
can dispense different levels of credits as it sees fit, for example to reflect
different buffer watermarks.

### Handshake

The handshake can be initiated from either side and is as follows. Certain
`REPORT` responses have been omitted for brevity:

1. The local side sends a `PROBE` message (`!@AB`) to indicate that it is in the
   disabled state.
2. The remote side responds with `!AAB` (enabled, no sessions) or `!BAB`
   (enabled, with existing sessions).
3. The local side sends a `REPORT` message confirming activation: `=!a@`.
4. If the remote side sent `!BAB`, the local side should send `;` to request a
   session restore.
   - The remote side sends a `RESTORE_START` message.
   - For each open session, the remote side sends a `OPEN_SESSION` message.
   - The remote side sends a `RESTORE_END` message.
5. If the remote side did not send `!BAB`, it may still send `OPEN_SESSION`
   messages to open sessions.
6. If the remote side did not open sessions, the local side may also request
   named or unnamed sessions be opened via `OPEN_SESSION` messages.
7. Either side may send a `SELECT_SESSION` message and start sending data once
   it receives an `ADD_CREDITS` message granting credits.

### Message Format

The stream is in "data mode" by default and all messages are directed towards
the selected session. To send a command to the remote side, send the intro byte
(`0x14`, a.k.a. `DC4`) followed by the opcode, parameters, and the term byte
(`0x1C`).

> "DC4 (0x14): Introduces an SSU session management command. The VT420 and host use this
> control to separate SSU commands from ANSI text and control functions" -- <https://manx-docs.org/collections/mds-199909/cd3/term/vt420rm2.pdf>

If a raw `0x14` is supposed to be sent, it is encoded as `0x14` `T` instead. XON
and XOFF to be sent to a session are similarily encoded as `0x14` `Q` and `0x14`
`S` respectively.

Parameters are encoded with an offset of 0x40, meaning that each character is
encoded as a six-bit value, with zero being `@`, one being `A`, etc.

Session IDs are encoded as 1-based indices, meaning that `A` is 1 and `B` is 2.
In certain cases, such as `RESET`, a session ID of 0 is used to indicate all
sessions.

The following opcodes are supported. Unrecognized opcodes should be ignored:

| Opcode | ASCII  | Opcode Name       | Description     |
| ------ | ------ | ----------------- | --------------- |
| `!`    | `0x21` | `PROBE`           | Probe/Enable    |
| `"`    | `0x22` | `OPEN_SESSION`    | Open session    |
| `#`    | `0x23` | `SELECT_SESSION`  | Select session  |
| ...    | ...    | ...               | ...             |
| `*`    | `0x2A` | `RESET`           | Reset           |
| `+`    | `0x2B` | `ADD_CREDITS`     | Add credits     |
| `,`    | `0x2C` | `UNUSED`          | (unused opcode) |
| `-`    | `0x2D` | `VERIFY_CREDITS`  | Verify credits  |
| `.`    | `0x2E` | `CLOSE_SESSION`   | Close session   |
| `/`    | `0x2F` | `DISABLE`         | Disable         |
| `0`    | `0x30` | `ZERO_CREDITS`    | Zero credits    |
| ...    | ...    | ...               | ...             |
| `:`    | `0x3A` | `SEND_BREAK`      | Send break      |
| `;`    | `0x3B` | `REQUEST_RESTORE` | Request restore |
| `<`    | `0x3C` | `RESTORE`         | Restore         |
| `=`    | `0x3D` | `REPORT`          | Report/Ack      |
| `>`    | `0x3E` | `RESTORE_END`     | Restore end     |
| `?`    | `0x3F` | `QUERY_SESSION`   | Query session   |

## Protocol Details

The following escape sequences are supported to encode control characters in a
way that avoids them being interpreted by the SSU-level server:

- To send a literal `0x11` (XON) to a session, send `0x14` `Q`.
- To send a literal `0x13` (XOFF) to a session, send `0x14` `S`.
- To send a literal `0x14` (DC4) to a session, send `0x14` `T`.

The VT420 will not interpret any other DC4-prefixed ASCII uppercase letters as
control characters, and they must be sent raw.

---

### Probe

Format: `!<x><y><z>`

Parameters:

- `<x>`: Current state (`@` disabled, `A` enabled, `B` enabled, but sessions
  exist)
- `<y>`: Protocol variant (`A`)
- `<z>`: Maximum number of sessions (`A` = 1, `B` = 2, etc.)

Initial probe message ("first enable"):

- `!@AB` ("I'm disabled, support protocol version 1 and, maximum 2 sessions")

Probe response message ("second enable"):

- `!AAB` ("I'm enabled, support protocol version 1 and, maximum 2 sessions")
- `!BAB` ("I'm enabled, have live sessions, support protocol version 1 and,
  maximum 2 sessions")

Response to second enable:

- `=!a@` ("OK")
- `=!ae` ("failed to enable")

Expected Responses:

When receiving a Probe, respond with a Probe message indicating current
state(`!AAB` for enabled, `!BAB` for enabled with sessions). The terminal side
will respond with a Report acknowledging Probe: `=!a@` ("OK").

If the remote side has live sessions, the terminal side should send `;` to
request a session restore.

---

### Open session

Format: `"<x><US><y><US>"`

Parameters:

- `<x>`: Session ID (A or B)
- `<y>`: Session name, surrounded by `US` (`0x1F`) bytes (or `@` for null name)

Expected Responses:

When receiving an Open message, respond with a `REPORT` acknowledging
`OPEN_SESSION`: `="<x>@` (where `<x>` is the session ID).

The receiver MAY also consider sending an `ADD_CREDITS` message for that session
at this point, and the sender may wish to grant credits once it has received the
report.

### Select session

Format: `#<x>`

Parameters:

- `<x>`: Session ID (A or B)

Expected Responses:

When receiving a Select message, respond with:

1. A Report acknowledging Select: `=#<x>@` (where `<x>` is the session ID).
2. The receiver MAY consider sending an `ADD_CREDITS` message for that session.

---

### Reset session

Performs a reset of all sessions or a specific session. This clears all buffers,
zeroes all credits, and resets the session to the initial state. If opened, the
session stays open.

This is sent when the terminal receives the Reset to Initial State (RIS) control
sequence.

Format: `*<x>`

Parameters:

- `<x>`: Session ID (`@` all sessions, or `A`, `B`, ...)

Expected Responses:

When receiving a Reset message, respond with a Report acknowledging Reset:
`=*<x>@` (where `<x>` is the session ID)

---

### Add credits

Format: `+<w><x><y><z>` or `+<w><y><z>`

Parameters:

- `<w>`: Session ID (`A` or `B`)
- `<x>`: 5 bits of credit data
- `<y>`: 5 bits of credit data
- `<z>`: 5 bits of credit data (0x40 bit must be set here, 0x20 bit is moved to
  high bit of credit)

Credits = `{ z5, x4, x3, x2, x1, x0, y4, y3, y2, y1, y0, z4, z3, z2, z1, z0 }`

Expected Responses:

When receiving an `ADD_CREDITS` message, no response is sent (credits are consumed
internally as data is sent). The receiver sending more data is implicit
acknowledgement of the `ADD_CREDITS` message.

---

### Verify credits

Sent when the local side runs out of credits, as an add credits message may have
been lost in transit. The sender must zero out its credit balance and wait for
more credits to be granted.

Format: `-<x>`

Parameters:

- `<x>`: Session ID (A or B)

Expected Responses:

When receiving a `VERIFY_CREDITS` message, respond with a `REPORT` acknowledging
`VERIFY_CREDITS`: `=-a@` ("OK") and optionally send an `ADD_CREDITS` message for
that session if there should be more credits available.

---

### Close session

Format: `.<x><y>`

Parameters:

- `<x>`: Session ID (A or B)
- `<y>`: Termination reason (`@` normal, `e` error)

Expected Responses:

When receiving a `CLOSE_SESSION` message, respond with a `REPORT` acknowledging
`CLOSE_SESSION`: `=.<x>@` (where `<x>` is the session ID)

---

### Disable session

Format: `/@@@`

Expected Responses:

When receiving a `DISABLE` message, respond with a `REPORT` acknowledging
`DISABLE`: `=/a@` ("OK")

---

### Zero credits

Instructs the remote side to zero out its credit balance for a specific session.
The remote side must not send any more data until it receives more credits.

Format: `0<x>`

Parameters:

- `<x>`: Session ID (A or B)

Expected Responses:

When receiving a `ZERO_CREDITS` message, respond with a `REPORT` acknowledging
`ZERO_CREDITS`: `=0<x>@` ("OK", where `<x>` is the session ID)

---

### Send break

Format: `:<x>`

Parameters:

- `<x>`: Session ID (A or B)

Expected Responses:

When receiving a `SEND_BREAK` message, no further response is sent (a serial
break signal of indeterminate duration is delivered to the session)

---

### Request restore

If a session has indicated that it has existing sessions, the local side may
request a session restore to give the remote side an opportunity to redraw the
terminal contents and restore the terminal state.

Format: `;`

Expected Responses:

- When receiving a RequestRestore message, respond with:
  1. A Report acknowledging RequestRestore: `=;a@`
  2. A `RESTORE_START` message (`<`)
  3. Optional open messages for each session to restore, followed by
     `SELECT_SESSION` messages and data transfer
  4. A `RESTORE_END` message (if last session)

---

### Restore start

Format: `<`

Expected Responses:

When receiving a Restore message, respond with a Report acknowledging Restore:
`=<a@` ("OK").

---

### Report

The `REPORT` message is used to acknowledge the successful receipt and/or
completion of an operation. It is sent in response to most of the above messages.

Format: `=<x><y><z>`

Parameters:

- `<x>`: Opcode being acknowledged
- `<y>`: Parameter (`a` seems to be used for "all", or session ID)
- `<z>`: Result code (`@` OK, `e` error)

Expected Responses:

When receiving a `REPORT` message, no further response is sent.

---

### Restore end

Format: `>`

Expected Responses:

When receiving a `RESTORE_END` message, respond with a `REPORT` acknowledging
`RESTORE_END`: `=>a@` ("OK")

---

### Query session

Format: `?<x>`

Parameters:

- `<x>`: Session ID (A or B)

Expected Responses:

Respond with either an OK or error report as appropriate: `=?<x>@` ("OK", where
`<x>` is the session ID) or `=?<x>e` ("ERROR", where `<x>` is the session ID)

## Escape Sequences

The host can send escape sequences to the terminal to query the current SSU
setup. These sequences are taken from the VT420 Programmer's Reference Manual:

`CSI ? 85 n`: The host asks for the status of the multiple-session configuration

`CSI ? 80 ; Ps2 n`: Multiple sessions are operating using the session support
utility (SSU) and the current SSU state is enabled. _Ps2_ indicates the maximum
number of sessions available. Default: _Ps2_ = 2.

`CSI ? 81 ; Ps2 n`: The terminal is currently configured for multiple sessions
using SSU but the current SSU state is pending. _Ps2_ indicates the maximum number
of sessions available. Default: _Ps2_ = 2.

`CSI ? 83 n`: The terminal is not configured for multiple-session operation.

`CSI ? 87 n`: Multiple sessions are operating using a separate physical line for
each session, not SSU.
