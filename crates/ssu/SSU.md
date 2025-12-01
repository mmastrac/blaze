# DEC SSU (Session Setup Utility) Protocol

a.k.a. DEC TD/SMP: Terminal Device/Session Management Protocol
(<https://en.wikipedia.org/wiki/TD/SMP>).

## What is it?

SSU is a protocol used to setup and manage multiple sessions on a terminal
device, multiplexed over a single physical connection.

## Message Format

The stream is in "data mode" by default and all messages are directed towards
the selected session. To send a command to the remote side, send the intro byte
(`0x14`, a.k.a. `DC4`) followed by the opcode, parameters, and the term byte
(`0x1C`).

If a raw `0x14` is supposed to be sent, it is encoded as `0x14` `T` instead. XON
and XOFF are similarily encoded as `0x14` `Q` and `0x14` `S` respectively.

Parameters are encoded with an offset of 0x40, meaning that each character is
encoded as a six-bit value, with zero being `@`, one being `A`, etc.

The following opcodes are supported:

| Opcode | ASCII  | Opcode Name       | Description     |
| ------ | ------ | ----------------- | --------------- |
| `!`    | `0x21` | `PROBE`           | Probe/Enable    |
| `"`    | `0x22` | `OPEN_SESSION`    | Open session    |
| `#`    | `0x23` | `SELECT_SESSION`  | Select session  |
| `*`    | `0x2A` | `RESET`           | Reset           |
| `+`    | `0x2B` | `ADD_CREDITS`     | Add credits     |
| `,`    | `0x2C` | `UNUSED`          | (unused opcode) |
| `-`    | `0x2D` | `VERIFY_CREDITS`  | Verify credits  |
| `.`    | `0x2E` | `CLOSE_SESSION`   | Close session   |
| `/`    | `0x2F` | `DISABLE`         | Disable         |
| `0`    | `0x30` | `ZERO_CREDITS`    | Zero credits    |
| `:`    | `0x3A` | `SEND_BREAK`      | Send break      |
| `;`    | `0x3B` | `REQUEST_RESTORE` | Request restore |
| `<`    | `0x3C` | `RESTORE`         | Restore         |
| `=`    | `0x3D` | `REPORT`          | Report/Ack      |
| `>`    | `0x3E` | `RESTORE_END`     | Restore end     |
| `?`    | `0x3F` | `QUERY_SESSION`   | Query session   |

## Escape Sequences

The following escape sequences are supported to encode control characters in a
way that avoids them being interpreted by the SSU-level server:

- To send a literal `0x11` (XON) to a session, send `0x14` `Q`.
- To send a literal `0x13` (XOFF) to a session, send `0x14` `S`.
- To send a literal `0x14` (DC4) to a session, send `0x14` `T`.

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
- `=!a<x>` ("failed to enable")

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

When receiving an Open message, respond with a Report acknowledging Open:
`="<x>@` (where `<x>` is the session ID). The receiver MAY also consider sending
an AddCredits message for that session.

### Select session

Format: `#<x>`

Parameters:

- `<x>`: Session ID (A or B)

Expected Responses:

When receiving a Select message, respond with:

1. A Report acknowledging Select: `=#<x>@` (where `<x>` is the session ID).
2. The receiver MAY consider sending an AddCredits message for that session.

---

### Reset session

Format: `*<x>`

Parameters:

- `<x>`: Session ID (A or B)

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

When receiving an AddCredits message, no response is sent (credits are consumed
internally as data is sent). The receiver sending more data is implicit
acknowledgement of the AddCredits message.

---

### Verify credits

Sent when the local side runs out of credits, as an add credits message may have
been lost in transit.

Format: `-<x>`

Parameters:

- `<x>`: Session ID (A or B)

Expected Responses:

When receiving a Verify message, respond with a Report acknowledging Verify:
`=-a@` ("OK") and optionally send an AddCredits message for that session if
there should be more credits available.

---

### Close session

Format: `.<x><y>`

Parameters:

- `<x>`: Session ID (A or B)
- `<y>`: Termination reason (`@` normal, `e` error)

Expected Responses:

When receiving a Close message, respond with a Report acknowledging Close:
`=.<x>@` (where `<x>` is the session ID)

---

### Disable session

Format: `/@@@`

Expected Responses:

When receiving a Disable message, respond with a Report acknowledging Disable:
`=/a@` ("OK")

---

### Zero credits

Format: `0<x>`

Parameters:

- `<x>`: Session ID (A or B)

Expected Responses:

When receiving a Zero message, respond with a Report acknowledging Zero:
`=0<x>@` ("OK", where `<x>` is the session ID)

---

### Send break

Format: `:<x>`

Parameters:

- `<x>`: Session ID (A or B)

Expected Responses:

When receiving a SendBreak message, no further response is sent (break is
delivered to the session)

---

### Request restore

Format: `;`

Expected Responses:

- When receiving a RequestRestore message, respond with:
  1. A Report acknowledging RequestRestore: `=;a@`
  2. A Restore message (`<`)
  3. Optional open messages for each session to restore
  4. A RestoreEnd message (if last session)

---

### Restore start

Format: `<`

Expected Responses:

When receiving a Restore message, respond with a Report acknowledging Restore:
`=<a@` ("OK").

---

### Response/Ack

Format: `=<x><y><z>`

Parameters:

- `<x>`: Opcode being acknowledged
- `<y>`: Parameter (`a` seems to be used for "all", or session ID)
- `<z>`: Result code (`@` OK, `e` error)

Expected Responses:

When receiving a Report message, no further response is sent.

---

### Restore end

Format: `>`

Expected Responses:

When receiving a RestoreEnd message, respond with a Report acknowledging
RestoreEnd: `=>a@`

---

### Query session

Format: `?<x>`

Parameters:

- `<x>`: Session ID (A or B)

Expected Responses:

Respond with either an OK or error report as appropriate: `=?<x>@` ("OK", where
`<x>` is the session ID) or `=?<x>e` ("ERROR", where `<x>` is the session ID)

## Protocol

The protocol is somewhat described in patent US5165020 ("Terminal Device/Session
Management Protocol") from 1991, but the details are omitted.

### Credits

Credits are used to track the available buffer space for the session. When a
side runs out of credits on a given channel, it must not send any more data
until it receives more credits.

Each side should preemptively add more credits as it detects the peer is running
low.

By default, each side of the session has no credits and must be granted credits
before sending data.

### Handshake

The handshake can be initiated from either side and is as follows:

1. The local side sends a `PROBE` message.
2. The remote side responds with `!AAB` or `!BAB`
3. The local side sends a `REPORT` message `=!a@`.
4. If the remote side sent `!BAB`, the local side should send `;` to request a
   session restore.
   - The remote side sends a `RESTORE_START` message.
   - For each open session, the remote side sends a `OPEN_SESSION` message.
   - The remote side sends a `RESTORE_END` message.
5. If the remote side did not send `!BAB`, it may send `OPEN_SESSION` messages
   to open sessions.
   - The remote side sends a `OPEN_SESSION` message.
6. The local side may request sessions be opened via `OPEN_SESSION` messages.
