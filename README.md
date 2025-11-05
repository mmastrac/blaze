# Blaze: an emulator for the VT420 terminal

Blaze is an emulator for the VT420 terminal. It is a work in progress and is not yet complete.

It is build on top of [the i8051 emulator crate](https://crates.io/crates/i8051).

The emulator includes a debugger and TUI for running the emulator with an
emulated display and keyboard.

## Features

- Emulates the VT420 terminal:
  - LK201 keyboard
  - DC7166B/DC7166C video processor
  - 8051 CPU
  - 5911 EEPROM
  - DUART (in progress)

## Quick Start

```
cargo run --release -- --rom roms/vt420/23-068E9-00.bin --display
```

## Screenshot

![Screenshot](docs/vt.gif)

## Debugging

Debugging is mutually exclusive with displaying the video RAM at this time.

```
# Set breakpoints at ABCD and 1ABCD
cargo run --release -- --rom roms/vt420/23-068E9-00.bin --debug --trace -v --bp ABCD --bp 1ABCD
```
