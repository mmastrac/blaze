# Architecture

## Components

 - CPU: 8051
 - ROM: 128kB (128k x 8bit)
 - Video/Memory Processor: DC7166B/DC7166C (custom part)
 - Video RAM: 128kB (dual-ported VRAM 256k x 4bit) OR 32kB VRAM + 32kB DRAM (64k x 4bit)
 - RAM: 32kB (32k x 8bit)
 - UART: 2681 DUART
 - UART Mux: 74LS157 (2 to 1 mux)
 - EEPROM: 5911 (128 x 8 or 64 x 16 serial EEPROM)
 - Keyboard: LK201/LK401

```mermaid

graph TD
    CPU --> ROM
    CPU --> MP[Video/Memory Processor]
    CPU --> Keyboard
    CPU -- R/W Strobe --> DUART
    CPU -- 232/423 --> Mux
    MP --> VR[Video RAM]
    MP --> SRAM
    MP --> DUART
    MP --> Video[Video Output]
    MP -- Upper 64k Select --> ROM
    DUART --> EEPROM
    DUART --> Mux
    Mux --> 423
    Mux --> 232
```

## CPU Ports

P1:

 - 7: Unused (doesn't match schematic)
 - 6: Program Enable (active low) - worldwide vs north american setting
 - 5: 232/423 Select (active high)
 - 4: DUART Reset (active high)
 - 0-3: Rotation Control (active high)

P2:

 - 0-7: Zero-page upper address bits 

P3:

 - 7: CPU Read Strobe (DUART, active low)
 - 6: CPU Write Strobe (DUART, active low)
 - 5: 60/70 Hz frame rate select (active high): connected to TEA2037A frame oscillator circuit (NOT used as timer/counter)
 - 4: CSYNC (VSYNC|HSYNC,active low) input: also connected to TEA2037A video input circuit (used as timer/counter input)
 - 3: DUART interrupt (active low)
 - 2: Memory Processor interrupt (active low): works like a CPU hold
 - 1: Keyboard TX
 - 0: Keyboard RX

## Memory Map

 - 0x0000-0x7fdf: VRAM (Addressable via "zero page" + P2 as well)
 - 0x7eXX-0x????: Possibly a mirror for the registers at 0x7ff0-0x7fff
 - 0x7fe0-0x7fef: DUART
 - 0x7ff0-0x7fff: Memory Processor Control
 - 0x8000-0xffff: SRAM

## DUART Ports

Port A:
 - Printer Receive/Transmit

Port B:
 - DUART Receive/Transmit (muxed to 232/423)

Input:

 - 6: Carrier Detect (active low)
 - 5: Printer Data Set Ready (active low)
 - 4: EEPROM Ready (active high)
 - 3: EEPROM Receive (active high)
 - 2: Speed Indicator (active low)
 - 1: Data Set Ready (active low)
 - 0: Clear to Send (active low)

Output:

 - 7: Printer Data Transmit Ready (active low)
 - 6: EEPROM Transmit (active high)
 - 5: EEPROM Clock (active high)
 - 4: EEPROM Chip Select (active high)
 - 3: Data Terminal Ready 2 (active low)
 - 2: Speed Select (active high)
 - 1: Data Terminal Ready 1 (active low)
 - 0: Ready To Send (active low)

## Memory Mapping Registers

 - 0x7ee4/0x7ee5: 16-bit register, copied to 0x7ff6 in two writes
 - 0x7ee6/0x7ee7: 16-bit register, copied to 0x7ffc in two writes

 - 0x7ef3:
  - `...._..xx` => x = VRAM page?
  - `...._.x..` => CMNCLK?
 - 0x7ef4: (same as 7ef3 but for session 2)
  - `...._..xx` => x = VRAM page?
  - `...._.x..` => CMNCLK?


 - 0x7ff3:
  - Set to `1010_0000` and then a delay - `1..._....` may be a reset
  - `1..._....` => blink register?
  - `..x._....` => VRAM page mapped at 0x8000?
  - `...x_....` => x = Some sort of swizzle? Could be used to quickly swap registers. Used for session flipping.
  - `...._x...` => x = screen register select?
  - `...._.xx.` => possibly invert
  - `...._...x` => 1 = 132 columns, 0 = 80 columns
  
 - 0x7ff4:
  - `x..._....` => 1 = 70Hz (70Hz ~14.29ms/frame, 536 lines), 0 = 60Hz (60Hz ~16.67ms/frame, 625 lines) (CONFIRMED via ROM disassembly)
  - `.x.x_....` => 01 = normal VRAM layout? 11 = alternate VRAM layout? (memory existance is tested in bootstrap, 11 is set if not there, 0x40 is toggled every ~1s, possibly for updating background page ram)
  - `...._..xx` => possibly invert/width
  - `...._x...` => possibly page flip control?
  
 - 0x7ff5:
  - `.x.._....` => x = alternate RAM layout?
  - `..x._....` => x = 0 = force SRAM mapping?
  - `...x_x...` => x = VRAM page select?
  - `...._.x..` => x = ROM bank select (CONFIRMED via ROM disassembly)
  
 - 0x7ff6: 2x 8-bit register, written twice, once for top and once for bottom half of screen
    - <a><b> - font height/row height (0 for 16px)
    - 78: 50 lines (0111_1000)
    - 9A: 38 lines (1001_1010)
    - D0: 26 lines (1101_0000)
    - F0/FC: (set during vsync: 1111_????, F0 = 24 lines, FC = other)

 - 0x7ff7/0x7ff8: screen offset (x/y), default 0x1e for each
    - x: 0x0a -> 0x32 (20px)
    - y: 0x01 -> 0x3b (60px)

## Video Timing

 - 60Hz: 16.67ms/frame, 625 lines, 417 active (208 vsync)
 - 70Hz: 14.29ms/frame, 536 lines, 417 active (119 vsync)

## Video RAM Layout

 - 0x0000-0x1fff: Screen 0
 - 0x2000-0x3fff: Screen 1

## Video RAM

 - 0x00, 0x01 ...: Per-row data
    - Byte 0:
        - `_______.` => memory page for row data
        - `.......x` => 1 = force 132 columns
    - Byte 1:
        - 0x02: split window divider
        - 0x04: double-width
        - 0x08: double-width, double-height top half
        - 0x0c: double-width, double-height bottom half
        - `......x.` => 1 = double width
        - `.......x` => 1 = swap between screen 0 and screen 1 attributes

 - Char attributes:
    
    0x02: bold
    0x04: reverse
    0x08: blink
    
    ?: underline
    ?: invisible