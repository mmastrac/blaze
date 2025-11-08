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

If the remote side has live sessions, the local side should send `;` to request
a session restore.

### Open session

Format: `"<x><US><y><US>"`

Parameters:

- `<x>`: Session ID (A or B)
- `<y>`: Session name, surrounded by `US` (`0x1F`) bytes (or `@` for null name)

### Select session

Format: `#<x>`

Parameters:

- `<x>`: Session ID (A or B)

### Reset session

Format: `*<x>`

Parameters:

- `<x>`: Session ID (A or B)

### Add credits

Format: `+<w><x><y><z>` or `+<w><y><z>`

Parameters:

- `<w>`: Session ID (`A` or `B`)
- `<x>`: 5 bits of credit data
- `<y>`: 5 bits of credit data
- `<z>`: 5 bits of credit data (0x40 bit must be set here, 0x10 bit is moved to
  high bit of credit)

Credits = `{ z5, x4, x3, x2, x1, x0, y4, y3, y2, y1, y0, z4, z3, z2, z1, z0 }`

### Verify credits

Format: `-<x>`

Parameters:

- `<x>`: Session ID (A or B)

### Close session

Format: `.<x><y>"`

Parameters:

- `<x>`: Session ID (A or B)
- `<y>`: Termination reason (`@` normal, `e` error)

### Disable session

Format: `/@@@`

Response:

- `=/a@` ("OK")

### Zero credits

Format: `0<x>`

Parameters:

- `<x>`: Session ID (A or B)

Response:

- `=0<x>@` ("OK")

### Request restore

Format: `;`

### Restore start

Format: `<`

### Response/Ack

Format: `=<x><y><z>`

Parameters:

- `<x>`: Opcode being acknowledged
- `<y>`: Parameter (`a` seems to be used for "all")
- `<z>`: Result code (`@` OK, `e` error)

### Restore end

Format: `>`

### Query session

Format: `?<x>`

Parameters:

- `<x>`: Session ID (A or B)

Response:

- `?<x>@` ("OK")

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
