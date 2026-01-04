# DEC SSU

This is an implementation of the DEC SSU protocol that includes a command-line
implementation for full operating systems.

While the SSU and session implementations are `async`, the crate avoids using a
specific I/O runtime, and is implemented in a way that it can easily be run on
an embedded system.
