# SSU Implementation — Bug Report

Review of `ops.rs`, `server.rs`, and `buffer.rs` against `SSU.md` (the
reverse-engineered DEC SSU / TD-SMP spec).

Findings are grouped by severity. Each entry has: where it lives, what's wrong,
the relevant spec clause, the impact, and a concrete fix. Line numbers refer to
the reviewed files and will drift as you edit.

| #  | Severity | Area                     | One-liner                                                          |
|----|----------|--------------------------|--------------------------------------------------------------------|
| 3  | High     | `server.rs` (`process_op`)| Peer-supplied `session_id` is unvalidated → index panic           |
| 4  | High     | `buffer.rs` / `WakerHandle`| Lost-wakeup race in async `push`/`pop` → per-session stall       |
| 5  | Medium   | `server.rs` (`ServerWrite`)| Blocking peer reader → head-of-line blocking across sessions     |
| 6  | Medium   | `server.rs` (`process_op`)| `CLOSE_SESSION` / `QUERY_SESSION` not acknowledged                |
| 7  | Medium   | `server.rs` (`process_op`)| `RESET` does not clear session buffers                            |
| 8  | Medium   | `ops.rs` (serialize)     | `AddCredits { credits: 0 }` emits no `<z>` byte                    |
| 9  | Low      | `server.rs` (`ServerWrite`)| DC4 escape decode accepts all `A–Z`, not just `Q/S/T`            |
| 10 | Low      | `ops.rs` / `process_op`  | Probe `max_sessions` negotiation uses `.max()`; unvalidated value  |
| 11 | Low      | `buffer.rs`              | `free()` overstates capacity by one (off-by-one)                   |
| 12 | Low      | `buffer.rs`              | `replace_with_slice` with `len == SIZE` breaks ring invariant      |
| 13 | Low      | `server.rs` (restore)    | Restore chain hangs if peer echoes an unexpected session id        |

---

---

## 3. Unvalidated peer `session_id` causes index panic — HIGH

**Location:** `server.rs`, `process_op` (and downstream `try_recv`,
`session_needs_credits`, the `write` data path).

**Problem.** Session ids arrive as `params[0].wrapping_sub(b'A')`, which can be
any value `0..=255`. Nothing checks the result against `max_sessions` /
`MAX_SESSION_COUNT` before indexing fixed-size arrays:

```rust
self.credits[session_id as usize].add(...);     // panics if >= 4
self.sessions[session_id as usize].push(b)...;  // panics if >= 4
```

Reachable from `Select`, `AddCredits`, `Zero`, `Reset(Some)`, `Verify`, and the
active-session data path (a `Select('Z')` sets `active_session_from_peer =
Some(25)`, then the next data byte panics).

**Spec.** *Message Format*:

> Session IDs are encoded as 1-based indices... Unrecognized opcodes should be
> ignored.

The spec is permissive about garbage; the implementation should not crash on it.
Since this protocol was reverse-engineered from hardware over a serial line,
malformed/half-received frames and line noise are expected.

**Impact.** Trivial remote denial of service: a single out-of-range id panics
the writer task (and, depending on the build, the process).

**Fix.** Validate centrally. Add a helper and reject early:

```rust
impl ServerWrite {
    fn valid_session(&self, id: u8) -> bool {
        (id as usize) < MAX_SESSION_COUNT && id < self.server.max_session()
    }
}
```

Then at the top of each arm that carries a `session_id`, bail out (optionally
replying with an error `Report`, e.g. `code: 1`, for opcodes the spec expects an
ack for):

```rust
SSUOp::Select(session_id) => {
    if !self.valid_session(session_id) {
        warn!("Select for invalid session {session_id}, ignoring");
        return;
    }
    // ...
}
```

Cleaner still: have `parse` reject ids outside a sane range, or return an
`Option`/`enum` the state machine must handle, so the invariant is enforced once
at the boundary.

---

---

## 5. Blocking peer reader → head-of-line blocking across sessions — MEDIUM

**Location:** `server.rs`, `ServerWrite::write`.

**Problem.** A single task drains the peer byte stream. On a data byte it does:

```rust
self.sessions[session_id as usize].push(b).await;
```

`push` is async and **blocks** when that session's `PEER_TO_SESSION` buffer is
full. While blocked, the task processes no further peer bytes — including
control commands and data destined for *other* sessions.

**Spec.** *Flow Control*:

> If the remote side continues sending data after running out of credits, the
> local side can send a `ZERO` message to force it to zero out its credit
> balance until there's enough buffer space.

The spec's remedy for an overrunning peer is `ZERO`, not blocking the link.

**Impact.** A slow consumer on one session stalls all sessions and command
processing. Credits make a full buffer rare (you grant ~15 KB into a ~16 KB
buffer), but they don't eliminate it, and the implementation never sends `ZERO`
to a peer that overruns its grant — so the one escape valve the spec provides is
unused.

**Fix.** Don't block the shared reader. Use a non-blocking push and apply
backpressure via the protocol instead:

```rust
if self.sessions[sid].push_sync_checked(b) {       // returns false if full
    // ok
} else {
    warn!("session {sid} buffer full; zeroing peer credits");
    self.credits[sid].zero(CreditsSlot::PeerToSession);
    self.outgoing_command_queue.push(SSUOp::Zero(sid)).await;
    // drop b, or stash a single byte; the peer must stop until re-credited
}
```

and only re-grant credits (`AddCredits`) once `free()` recovers above the low
water mark. This keeps the reader live for every other session and matches the
credit/`ZERO` contract.

---

## 6. `CLOSE_SESSION` and `QUERY_SESSION` are not acknowledged — MEDIUM

**Location:** `server.rs`, `process_op` — both fall through to the `_ =>`
warn arm.

**Spec.**

> *Close session*: respond with a `REPORT` acknowledging `CLOSE_SESSION`:
> `=.<x>@`.
>
> *Query session*: respond with `=?<x>@` ("OK") or `=?<x>e` ("ERROR").

**Impact.** A peer that closes or queries a session waits for an ack that never
arrives. Depending on the peer's state machine this can stall teardown or
status polling.

**Fix.** Add explicit arms:

```rust
SSUOp::Close(session_id, _reason) => {
    // tear down local state for the session as appropriate
    self.outgoing_command_queue.push(SSUOp::Report {
        op: SSUOpcode::Close,
        session_id: Some(session_id),
        code: 0,
    }).await;
}
SSUOp::Query(session_id) => {
    let ok = self.valid_session(session_id); // see #3
    self.outgoing_command_queue.push(SSUOp::Report {
        op: SSUOpcode::Query,
        session_id: Some(session_id),
        code: if ok { 0 } else { 1 }, // '@' OK / 'e' ERROR
    }).await;
}
```

(`SEND_BREAK` legitimately needs no reply per spec, but you currently also drop
the break itself rather than delivering it to the session — worth a follow-up if
break semantics matter. Incoming `RESTORE_START` / `RESTORE_END` are
role-dependent since this side initiates restore.)

---

## 7. `RESET` does not clear session buffers — MEDIUM

**Location:** `server.rs`, `process_op`, `SSUOp::Reset` arm.

**Problem.** The handler zeroes credits but leaves the session ring buffers
populated:

```rust
SSUOp::Reset(session_id) => {
    // ... Report ack ...
    if let Some(session_id) = session_id {
        self.credits[session_id as usize].zero_all();
    } else {
        for i in 0..self.server.max_session() {
            self.credits[i as usize].zero_all();
        }
    }
}
```

Compare the `Probe(Disabled)` arm, which *does* `session.clear()` — the omission
here looks accidental.

**Spec.** *Reset session*:

> This clears all buffers, zeroes all credits, and resets the session to the
> initial state.

**Impact.** After a RESET (sent on the terminal's RIS / Reset-to-Initial-State),
stale bytes remain queued and will be delivered after reset, corrupting the
post-reset stream.

**Fix.** Clear the corresponding buffer(s) alongside the credit zeroing. Note you
must clear both directions for the affected session(s) — the `PEER_TO_SESSION`
buffer held by `ServerWrite` and the `SESSION_TO_PEER` buffer drained by
`ServerRead` (the latter is reachable via the shared handle / credits wiring):

```rust
if let Some(sid) = session_id {
    self.sessions[sid as usize].clear();
    self.credits[sid as usize].zero_all();
} else {
    for i in 0..self.server.max_session() as usize {
        self.sessions[i].clear();
        self.credits[i].zero_all();
    }
}
```

---

## 8. `AddCredits { credits: 0 }` serializes without the required `<z>` — MEDIUM

**Location:** `ops.rs`, `serialize`, `OP_ADDCR` arm.

**Problem.** The emit cascade produces no parameter bytes when all of `x`, `y`,
`z` are zero:

```rust
if x > 0 { /* emit x */ }
if x > 0 || y > 0 { /* emit y */ }
if x > 0 || y > 0 || z > 0 { /* emit z */ }
```

For `credits == 0` the frame is `INTRO '+' <sid> TERM` with no credit byte.

**Spec.** *Add credits*:

> The `<z>` parameter is required and always indicated by having a value
> `>= 0x40`.

**Impact.** Although it round-trips through this implementation's own parser
(`params.len() == 1 => (0, 0, 0)`), a conformant terminal may reject the frame as
malformed. Reachable from the `Verify` handler when `free()` is near zero.

**Fix.** Always emit at least the `<z>` byte:

```rust
if x > 0 {
    buf[pos] = x + b' ';
    pos += 1;
}
if x > 0 || y > 0 {
    buf[pos] = y + b' ';
    pos += 1;
}
// z is mandatory and always present (>= 0x40 thanks to the '@' bias)
buf[pos] = z + b'@';
pos += 1;
```

The round-trip test in #2 covers this once `credits == 0` is in range.

---

## 9. DC4 escape decode accepts all `A–Z`, not just `Q/S/T` — LOW

**Location:** `server.rs`, `ServerWrite::write`, the `_ =>` byte arm.

**Problem.**

```rust
if self.incoming_command_queue.len() == 1 && (b'A'..=b'Z').contains(&b) {
    self.incoming_command_queue.clear();
    b = b.saturating_sub(b'@');
}
```

Every uppercase letter after `0x14` is turned into `b - 0x40`. So `0x14 A`
becomes `0x01`, `0x14 X` becomes `0x18`, etc.

**Spec.** *Protocol Details*:

> The VT420 will not interpret any other DC4-prefixed ASCII uppercase letters as
> control characters, and they must be sent raw.

Only `Q`, `S`, `T` are escapes.

**Impact.** Mostly latent, because a conformant peer only emits `Q/S/T` escapes
and opcodes occupy `0x21–0x3F` (no overlap with `A–Z`). But a peer that sends
`0x14 <other-uppercase>` will have those bytes silently mis-decoded into control
codes instead of passed through.

**Fix.** Match only the defined escapes:

```rust
if self.incoming_command_queue.len() == 1 {
    let decoded = match b {
        b'Q' => Some(0x11),
        b'S' => Some(0x13),
        b'T' => 0x14.into(),
        _ => None,
    };
    if let Some(d) = decoded {
        self.incoming_command_queue.clear();
        b = d;
    }
}
```

Decide explicitly what "sent raw" means for other uppercase letters in your
peer's dialect (pass `0x14` then the letter through, or drop) and document it.

---

## 10. Probe `max_sessions` negotiation uses `.max()`; value unvalidated — LOW

**Location:** `server.rs`, `process_op` Probe arms; `ops.rs` `serialize` Probe.

**Problems.**

1. Capability negotiation takes the **max** of the two sides' limits:

   ```rust
   let max_sessions = max_sessions.max(self.server.max_session());
   ```

   Negotiation normally converges on the **min** so neither side is asked to
   handle more sessions than it supports.

2. The peer's `max_sessions` flows unvalidated into `serialize`:

   ```rust
   buf[pos] = *max_sessions + b'@';
   ```

   A large peer value (e.g. from line noise) can overflow `u8` and panic in debug
   builds — same untrusted-input theme as #3.

3. The server always advertises `EnabledWithSessions` even when no sessions
   exist, which obliges a spec-compliant host to issue `REQUEST_RESTORE` for
   nothing. This appears intentional (you synthesize the restore), but it's worth
   confirming against real terminal behavior.

**Spec.** *Probe* defines the `max_sessions` parameter and the `!AAB` /
`!BAB` responses but does not pin down the negotiation rule, so this is a
judgment call — flagged for confirmation, not a hard violation.

**Fix.** Prefer `.min()` and clamp:

```rust
let negotiated = max_sessions
    .min(self.server.max_session())
    .min(MAX_SESSION_COUNT as u8);
```

and reject absurd peer values during parse.

---

## 11. `free()` overstates capacity by one — LOW

**Location:** `buffer.rs`.

**Problem.** The ring reserves one empty slot (`is_full` is
`(write + 1) % SIZE == read`), so usable capacity is `SIZE - 1` and `len()` maxes
at `SIZE - 1`. `free()` is `SIZE - len()`, which therefore returns `1` when the
buffer is actually full.

**Impact.** In the `Verify` handler, `let remaining = self.sessions[sid].free();`
can grant one phantom credit when the buffer has no real space.

**Fix.** Account for the reserved slot:

```rust
pub fn free(&self) -> usize {
    (SIZE - 1) - self.len()
}
```

or document the off-by-one and subtract one at the call site.

---

## 12. `replace_with_slice` with `len == SIZE` breaks the ring invariant — LOW

**Location:** `buffer.rs`, `SyncRingBuffer::replace_with_slice`.

**Problem.**

```rust
pub fn replace_with_slice(&mut self, op_buf: &[T]) {
    assert!(op_buf.len() <= SIZE);
    self.read_index = 0;
    self.write_index = op_buf.len();
    // ...
}
```

If `op_buf.len() == SIZE`, `write_index` becomes `SIZE`, which is outside the
valid `[0, SIZE)` modular range. `pop` increments `read_index` modulo `SIZE` and
will never reach `write_index == SIZE`, so the buffer never reports empty.

**Impact.** Unreachable today (serialized commands are well under
`MAX_COMMAND_LEN = 128`), but a latent footgun if `MAX_COMMAND_LEN` ever shrinks
or a longer command type is added.

**Fix.** Tighten the assert to the real capacity:

```rust
assert!(op_buf.len() < SIZE, "replace_with_slice exceeds ring capacity");
```

---

## 13. Restore chain hangs on an unexpected ack session id — LOW

**Location:** `server.rs`, `process_op`, `Report { op: Open, session_id, .. }`.

**Problem.** The restore sequence advances only on
`Report { op: Open, session_id: Some(id), .. }` and sets `enabled = true` only
after `Report { op: RestoreEnd, .. }`. If the peer acks an Open with
`session_id: None` (`'a'`) or an unexpected id, the message falls into the `_ =>`
arm, the chain stalls, and `enabled` stays `false` forever — the data path never
opens.

**Impact.** A single off-pattern ack silently wedges the session in the
disabled state with no recovery.

**Fix.** Loosen the match (advance on any Open ack), and/or add a restore
timeout that re-issues the Open or aborts to a known state. At minimum, log at
`warn`/`error` when an Open ack doesn't match the expected id so the stall is
diagnosable rather than silent.

---

## Suggested order of work

1. **#3** (validate session ids) and **#1** (outbound escaping) — these are the
   ones most likely to bite against real hardware: a crash and silent data
   corruption respectively.
2. **#4** (waker race) — latent but ugly when it triggers; the under-lock
   registration fix is small.
3. **#2** + **#8** — fix both, then land the `add_credits_roundtrip` proptest so
   they can't regress.
4. **#5**, **#6**, **#7** — protocol-conformance and robustness.
5. The LOW items as cleanup.

A `parse(serialize(op)) == op` property test across all opcodes (not just
credits) plus a Loom or multi-threaded stress test on the buffer would close the
gaps that the current unit tests leave open.
