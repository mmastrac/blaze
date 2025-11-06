use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use clap::Parser;
use i8051::{ControlFlow, Cpu, CpuContext, Opcode, ReadOnlyMemoryMapper, memory::ROM};

/// VT420 Terminal Emulator
/// Emulates a VT420 terminal using an 8051 microcontroller
#[derive(Parser)]
#[command(name = "vt-emulator")]
#[command(about = "A VT420 terminal emulator using 8051 CPU emulation")]
struct Args {
    /// Path to the ROM file
    #[arg(short, long)]
    rom: PathBuf,

    /// Output path
    #[arg(short, long)]
    output: PathBuf,

    /// Enable debug output
    #[arg(long)]
    debug: bool,
}

/// Simple context for disassembly that only provides ROM access
struct DisassemblyContext {
    rom: ROM,
    ports: (),
    xdata: (),
}

impl CpuContext for DisassemblyContext {
    type Ports = ();
    type Xdata = ();
    type Code = ROM;

    fn ports(&self) -> &Self::Ports {
        &self.ports
    }
    fn xdata(&self) -> &Self::Xdata {
        &self.xdata
    }
    fn code(&self) -> &Self::Code {
        &self.rom
    }
    fn ports_mut(&mut self) -> &mut Self::Ports {
        &mut self.ports
    }
    fn xdata_mut(&mut self) -> &mut Self::Xdata {
        &mut self.xdata
    }
    fn code_mut(&mut self) -> &mut Self::Code {
        &mut self.rom
    }
}

pub fn main() {
    let args = Args::parse();
    let rom = fs::read(&args.rom).unwrap();
    fs::create_dir_all(&args.output).unwrap();
    disassemble(&rom[0..0x10000], &args.output.join("bank0.asm"), args.debug).unwrap();
    // disassemble(&rom[0x10000..], &args.output.join("bank1.asm")).unwrap();
}

#[derive(Debug, Clone, Default)]
enum AddressState {
    #[default]
    Unknown,
    Data,
    InstructionStart {
        root: bool,
        jump_target: bool,
        addrs: BTreeSet<u16>,
    },
    InstructionContinue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flow {
    Root,
    Step,
    Jump,
}

fn disassemble(rom: &[u8], output: &Path, debug: bool) -> io::Result<()> {
    let mut file = fs::File::create(output)?;
    let mut roots: Vec<(Flow, u16, u16)> = vec![];

    let mut address_state = Vec::with_capacity(65536);
    address_state.extend(std::iter::repeat(AddressState::default()).take(65536));

    // Add the 8051 interrupt vectors
    roots.push((Flow::Root, 0x0000, 0x0000));
    roots.push((Flow::Root, 0x0003, 0x0003));
    roots.push((Flow::Root, 0x000B, 0x000B));
    roots.push((Flow::Root, 0x0013, 0x0013));
    roots.push((Flow::Root, 0x001B, 0x001B));
    roots.push((Flow::Root, 0x0023, 0x0023));

    for bank_switch in 0..0x1e {
        let lo = rom[0x100 + bank_switch * 2];
        let hi = rom[0x101 + bank_switch * 2];
        address_state[0x100 + bank_switch * 2] = AddressState::Data;
        address_state[0x101 + bank_switch * 2] = AddressState::Data;
        let pc = (hi as u16) << 8 | (lo as u16);
        roots.push((Flow::Root, pc, pc));
    }

    // Locate all cross-bank thunks
    for (pc, _) in rom
        .windows(5)
        .enumerate()
        .filter(|(_, window)| window[0] == 0x74 && window[2] == 0x02 && window[3] == 0)
    {
        println!("Root: thunk at 0x{:04X}", pc);
        roots.push((Flow::Root, pc as u16, pc as u16));
    }

    let cpu = Cpu::new();
    let ctx = DisassemblyContext {
        rom: ROM::new(rom.to_vec()),
        ports: (),
        xdata: (),
    };

    loop {
        while let Some(root) = roots.first_mut() {
            let flow = root.0;
            let jump_target = flow == Flow::Jump;
            let prev = root.1;
            let pc = root.2;
            match &mut address_state[pc as usize] {
                AddressState::Data => {
                    println!("WARNING: Data at 0x{:04X}", pc);
                    roots.remove(0);
                    continue;
                }
                AddressState::InstructionContinue => {
                    println!("WARNING: Instruction decoded from middle at 0x{:04X}", pc);

                    let mut chain = vec![pc, prev];
                    let mut current = prev;
                    // Walk the chain of reachability to a root
                    loop {
                        let AddressState::InstructionStart { root, addrs, .. } =
                            &mut address_state[current as usize]
                        else {
                            println!(
                                "WARNING: Could not get roots from 0x{:04X}, {:?}",
                                current, address_state[current as usize]
                            );
                            break;
                        };
                        if *root {
                            break;
                        }
                        let next = *addrs.iter().find(|&&a| a != current).unwrap_or_else(|| {
                            panic!("No next address, only found {:04X?}", addrs)
                        });
                        chain.push(next);
                        current = next;
                    }
                    println!("WARNING:   addrs = {:04X?}", chain);
                    roots.remove(0);
                    continue;
                }
                AddressState::InstructionStart {
                    jump_target, addrs, ..
                } => {
                    // Already decoded
                    addrs.insert(prev);
                    if flow == Flow::Jump {
                        *jump_target = true;
                    }
                    roots.remove(0);
                    continue;
                }
                AddressState::Unknown => {
                    // Not yet decoded
                }
            }

            let instruction = cpu.decode(&ctx, pc as u32);
            if debug {
                println!("{:#}", instruction);
            }
            if instruction.mnemonic() == Opcode::Unknown {
                println!("WARNING: Unknown instruction at 0x{:04X}", pc);
                roots.remove(0);
                continue;
            }

            address_state[pc as usize] = if prev == pc {
                AddressState::InstructionStart {
                    root: true,
                    jump_target,
                    addrs: BTreeSet::from_iter([]),
                }
            } else {
                AddressState::InstructionStart {
                    root: false,
                    jump_target,
                    addrs: BTreeSet::from_iter([prev]),
                }
            };
            for i in 1..instruction.len() {
                if matches!(address_state[pc as usize + i], AddressState::Unknown) {
                    address_state[pc as usize + i] = AddressState::InstructionContinue;
                } else {
                    println!("WARNING: Already decoded at 0x{:04X}", pc as usize + i);
                }
            }

            let curr_pc = pc;
            let flow_pc = pc + instruction.len() as u16;
            match instruction.control_flow() {
                ControlFlow::Continue(pc) => {
                    if pc != curr_pc {
                        root.0 = if pc == flow_pc {
                            Flow::Step
                        } else {
                            Flow::Jump
                        };
                        root.1 = root.2;
                        root.2 = pc;
                    }
                }
                ControlFlow::Call(next, jmp) => {
                    root.0 = if next == flow_pc {
                        Flow::Step
                    } else {
                        Flow::Jump
                    };
                    root.1 = root.2;
                    root.2 = next;
                    if debug {
                        println!("-> Adding {jmp:04X}");
                    }
                    roots.push((Flow::Jump, pc, jmp));
                }
                ControlFlow::Choice(pc1, pc2) => {
                    root.0 = if pc1 == flow_pc {
                        Flow::Step
                    } else {
                        Flow::Jump
                    };
                    root.1 = root.2;
                    root.2 = pc1;
                    if debug {
                        println!("-> Adding {pc2:04X}");
                    }
                    if pc2 != curr_pc {
                        roots.push((Flow::Jump, pc, pc2));
                    }
                }
                ControlFlow::Diverge => {
                    roots.remove(0);
                }
            }
        }

        let mut is_unknown = 0;
        let mut is_code = 0;
        for (i, state) in address_state.iter().enumerate() {
            match state {
                AddressState::Unknown => {
                    if rom[i] != 0xff {
                        is_unknown += 1
                    }
                }
                AddressState::InstructionStart { .. } => is_code += 1,
                AddressState::InstructionContinue => is_code += 1,
                AddressState::Data => {}
            }
        }

        println!("Unknown: {is_unknown}");
        println!("Code: {is_code}");

        let mut unknown_calls = BTreeMap::new();
        for (i, state) in address_state.iter().enumerate() {
            match state {
                AddressState::Unknown => {
                    if rom[i] != 0xff {
                        let instruction = cpu.decode(&ctx, i as u32);
                        if let Some(addr) = instruction.addr() {
                            if matches!(address_state[addr as usize], AddressState::Unknown) {
                                if addr > 0x100 && rom[addr as usize] != 0xff {
                                    if matches!(
                                        instruction.mnemonic(),
                                        Opcode::ACALL | Opcode::LCALL | Opcode::LJMP | Opcode::AJMP
                                    ) {
                                        unknown_calls
                                            .entry(addr)
                                            .or_insert(vec![])
                                            .push(instruction);
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        for (addr, instructions) in unknown_calls.iter() {
            let count = instructions.len();
            if count > 5 {
                println!("Unknown call to {addr:04X} ({count} times):");
                for instruction in instructions {
                    println!("  {:#}", instruction);
                }
                roots.push((Flow::Root, *addr, *addr));
            }
        }

        // Locate common code patterns
        for (pc, window) in rom
            .windows(5)
            .enumerate()
            .filter(|(_, window)| {
                window[0] == 0xc0
                    && (window[1] == 0x82 || window[1] == 0x83)
                    && window[2] == 0xc0
                    && (window[3] == 0x82 || window[3] == 0x83)
                    && window[4] == 0x90
            })
            .filter(|(pc, _)| matches!(address_state[*pc as usize], AddressState::Unknown))
        {
            println!(
                "Root: common code pattern (PUSH DPx, PUSH DPx, MOV DPTR) at 0x{:04X}: {:04X?}",
                pc, window
            );
            roots.push((Flow::Root, pc as u16, pc as u16));
        }

        // Locate common code patterns
        for (pc, window) in rom
            .windows(5)
            .enumerate()
            .filter(|(_, window)| {
                window[0] == 0xc0
                    && (window[1] == 0x82 || window[1] == 0x83)
                    && window[2] == 0xc0
                    && (window[3] == 0x82 || window[3] == 0x83)
                    && window[4] == 0x90
            })
            .filter(|(pc, _)| matches!(address_state[*pc as usize], AddressState::Unknown))
        {
            println!(
                "Root: common code pattern (PUSH DPx, PUSH DPx, MOV DPTR) at 0x{:04X}: {:02X?}",
                pc, window
            );
            roots.push((Flow::Root, pc as u16, pc as u16));
        }

        // Locate common code patterns
        for (pc, window) in rom
            .windows(4)
            .enumerate()
            .filter(|(_, window)| window[0] == 0x90 && window[1] == 0x7f && window[3] == 0xe0)
            .filter(|(pc, _)| matches!(address_state[*pc as usize], AddressState::Unknown))
        {
            println!(
                "Root: common code pattern (MOV DPTR, 0x7fxx, MOVX A, @DPTR) at 0x{:04X}: {:02X?}",
                pc, window
            );
            roots.push((Flow::Root, pc as u16, pc as u16));
        }

        if roots.is_empty() {
            break;
        }
    }

    let mut pc = 0_u16;
    loop {
        match address_state[pc as usize] {
            AddressState::Unknown | AddressState::Data => {
                writeln!(file, "  DATA {:02X}", ctx.rom.read(&(&cpu, &ctx), pc as u32))?;
                pc = pc.wrapping_add(1);
            }
            AddressState::InstructionStart {
                jump_target, root, ..
            } => {
                let instruction = cpu.decode(&ctx, pc as u32);
                if jump_target {
                    writeln!(file, "label_{pc:04X}:")?;
                } else if root {
                    writeln!(file, "root_{pc:04X}:")?;
                }
                writeln!(file, "  {}", instruction)?;
                pc = pc.wrapping_add(instruction.len() as u16);
            }
            _ => {}
        }
        if pc == 0 {
            break;
        }
    }

    Ok(())
}
