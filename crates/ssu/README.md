# DEC SSU

This is an implementation of the DEC SSU protocol that includes a command-line
implementation for full operating systems.

While the SSU and session implementations are `async`, the crate avoids using a
specific I/O runtime, and is implemented in a way that it can easily be run on
an embedded system.

## Protocol

The protocol is somewhat described in patent US5165020 ("Terminal Device/Session
Management Protocol") from 1991, but the details are omitted and were
painstakingly reverse engineered from a real VT420 terminal.

See <https://github.com/mmastrac/blaze/blob/main/architecture/SSU.md> for
detailed protocol information.

### Flow Control

SSU uses a simple flow control mechanism based on credits. Each side is
allocated a number of credits to use for sending data, and the strategy for
dispensing them is left to the implementation. Generally, the credits will
reflect the available buffer space for the session.

### Message Format

The stream is in "data mode" by default and all messages are directed towards
the selected session. To send a command to the remote side, send the intro byte
(`0x14`, a.k.a. `DC4`) followed by the opcode, parameters, and the term byte
(`0x1C`).

The following opcodes are supported:

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

## Usage

Run the server with a shell on session "A".

```
cargo run -p ssu --all-features -- --session 'exec /bin/sh'
```

Note that running the server through [`blaze`](https://crates.io/crates/blaze)
is the intended use case. In that case, start the server inside the terminal
emulator, then configure the terminal's communication port to use "Sessions on
Comm1" from the "Global" menu and then select "Enable Sessions" from the main setup menu.

```
blaze-vt --display graphics --rom <rom> --comm1 "exec 'ssu --session \'exec /bin/sh\''"
```
