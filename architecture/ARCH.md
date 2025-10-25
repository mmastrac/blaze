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
 - 5: 60/70 Hz Pulse (active high)
 - 4: CSYNC (active low)
 - 3: DUART interrupt (active low)
 - 2: Memory Processor interrupt (active low)
 - 1: Keyboard TX
 - 0: Keyboard RX

## Memory Map

 - 0x0000-0x7fdf: VRAM (Addressable via "zero page" + P2 as well)
 - 0x7eXX-0x????: 
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

 - 0x7ef3:
  - `...._..xx` => x = VRAM page?
  - `...._.x..` => CMNCLK?

 - 0x7ff3:
  - Set to `1010_0000` and then a delay
  - `...x_....` => x = Some sort of swizzle? Could be used to quickly swap registers.
  - `...._..xx` => possibly invert/width
  
 - 0x7ff4:
  - `.x.._....` => x = alternate RAM layout?
  - `...._..xx` => possibly invert/width
  - `...._x...` => possibly page flip control?
  
 - 0x7ff5:
  - `.x.._....` => x = alternate RAM layout?
  - `..x._....` => x = enable SRAM at 0x8000? Might also be enabling DUART mapping?
  - `...._.x..` => x = ROM bank select
  