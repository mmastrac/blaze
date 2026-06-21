use std::collections::HashSet;
use std::fmt;
use std::io;

use i8051_disassembler::address::AddressValue;
use i8051_disassembler::db::EquivalentAt;
use i8051_disassembler::db::EquivalentKind;
use i8051_disassembler::pattern::BytePattern;
use i8051_disassembler::{
    address::{AddressSpace, PhysicalAddr},
    db::{DataType, Db, Equivalent, Function},
    region::Region,
};

const BANK_SIZE: usize = 0x1_0000;
const ROM_SIZE: usize = 2 * BANK_SIZE;
const JUMP_TABLE_BASE: u32 = 0x0100;
const THUNK_SEARCH_LIMIT: usize = 0x250;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Bank {
    Bank0,
    Bank1,
}

impl fmt::Display for Bank {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "bank{}", *self as u8)
    }
}

impl Bank {
    const BANKS: [Bank; 2] = [Bank::Bank0, Bank::Bank1];

    pub const fn base(self) -> u32 {
        match self {
            Bank::Bank0 => 0,
            Bank::Bank1 => BANK_SIZE as u32,
        }
    }

    pub const fn other(self) -> Self {
        match self {
            Bank::Bank0 => Bank::Bank1,
            Bank::Bank1 => Bank::Bank0,
        }
    }

    pub const fn for_addr(addr: AddressValue) -> Bank {
        if addr < BANK_SIZE as u32 {
            Bank::Bank0
        } else {
            Bank::Bank1
        }
    }
}

fn fmt_addr(addr: AddressValue) -> String {
    format!("0x{addr:05X}")
}

/// 8051 hardware interrupt vector addresses (each bank has its own copy at these offsets).
const INTERRUPT_VECTORS: &[(u32, &'static str)] = &[
    (0x0000, "RESET"),
    (0x0003, "INT0"),
    (0x000B, "TIMER0"),
    (0x0013, "INT1"),
    (0x001B, "TIMER1"),
    (0x0023, "SERIAL"),
];

#[derive(Default)]
pub struct RomStaticAnalysisInfo {
    db: Db,
    thunks: [Vec<Thunk>; 2],
    jump_tables: [JumpTableStaticAnalysisInfo; 2],
}

#[derive(Default, Clone)]
pub struct JumpTableFunctionInfo {
    addr: AddressValue,
    length: AddressValue,
    tables: Vec<JumpTableInfo>,
}

#[derive(Default, Clone)]
pub struct JumpTableInfo {
    base: AddressValue,
    range: Option<(AddressValue, AddressValue)>,
}

#[derive(Default)]
pub struct JumpTableStaticAnalysisInfo {
    functions: Vec<JumpTableFunctionInfo>,
}

/// A VT420 cross-bank dispatch stub and its resolved target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thunk {
    /// Dispatch table index (`74 <id> 02 00 …` operand).
    id: u8,
    /// Cross-bank stub entry point (absolute ROM offset).
    thunk_addr: u32,
    /// Target function in the other bank (absolute ROM offset).
    target_addr: u32,
}

pub fn load_rom(
    rom: Vec<u8>,
    additional_roms: Vec<RomStaticAnalysisInfo>,
) -> Result<RomStaticAnalysisInfo, io::Error> {
    let mut info = RomStaticAnalysisInfo::default();

    let db = &mut info.db;
    let region = db.region_mut(AddressSpace::Code);
    region.set_bytes("rom", 0, 0, &rom);

    for bank in Bank::BANKS {
        info.thunks[bank as usize] = detect_thunks(region, bank, bank.other());
        let additional_rom_jump_tables = additional_roms
            .iter()
            .map(|rom| {
                (
                    rom.db.region(AddressSpace::Code).unwrap(),
                    &rom.jump_tables[bank as usize],
                )
            })
            .collect::<Vec<_>>();
        info.jump_tables[bank as usize] =
            detect_jump_tables(region, bank, additional_rom_jump_tables.as_slice());
    }

    Ok(info)
}

pub fn auto_analyze(mut info: RomStaticAnalysisInfo, verbose: bool) -> Result<Db, io::Error> {
    let db = &mut info.db;
    let region = db.region_mut(AddressSpace::Code);
    for bank in Bank::BANKS {
        mark_interrupts(region, bank);
        mark_thunks(region, bank, &info.thunks[bank as usize]);
        let entry_count = info.thunks[bank.other() as usize]
            .iter()
            .map(|thunk| thunk.id)
            .max()
            .unwrap_or(0) as usize
            + 1;
        mark_thunk_jump_table(region, bank, entry_count);
        mark_jump_tables(region, bank, &info.jump_tables[bank as usize]);
    }

    Ok(info.db)
}

pub fn process_heuristics(db: &mut Db, pc_trace: Option<Vec<u8>>) -> Result<(), io::Error> {
    let pc_trace = pc_trace.as_deref();
    let region = db.region_mut(AddressSpace::Code);
    if let Some(trace) = pc_trace {
        let mut new_roots = 0usize;
        for (addr, &marked) in trace.iter().enumerate() {
            if marked == 0 {
                continue;
            }
            let addr = addr as u32;
            if let Some(kind) = region.get_equivalent_kind(addr) {
                if kind != EquivalentKind::Code {
                    eprintln!("  WARNING: overlapped {kind:?} at {}", fmt_addr(addr));
                }
            } else {
                for error in region.auto_disassemble(addr).errors {
                    eprintln!("  ERROR: {error:?}");
                }
                new_roots += 1;
                region.set_comment(addr, &format!("pc-trace root ({})", fmt_addr(addr)));
            }
        }
        eprintln!("pc-trace: {new_roots} new roots");
    }
    let push_roots = {
        let region = db.region(AddressSpace::Code).unwrap();
        push_dpx_mov_dptr_roots(region)
    };
    let mov_7fxx_roots = {
        let region = db.region(AddressSpace::Code).unwrap();
        mov_dptr_7fxx_movx_roots(region)
    };
    let mov_2x_roots = {
        let region = db.region(AddressSpace::Code).unwrap();
        mov_dptr_2x_roots(region)
    };
    let region = db.region_mut(AddressSpace::Code);
    apply_heuristic("PUSH DPx, PUSH DPx, MOV DPTR", region, push_roots);
    apply_heuristic("MOV DPTR, 0x7fxx, MOVX A, @DPTR", region, mov_7fxx_roots);
    apply_heuristic("MOV DPTR 2x", region, mov_2x_roots);
    let usage = db.space_usage(AddressSpace::Code);
    eprintln!(
        "rom: code={} data={} undefined={} (total={})",
        usage.code,
        usage.data,
        usage.undefined,
        usage.total(),
    );
    Ok(())
}

/// Find cross-bank dispatch stubs (`74 <id> 02 00`) and resolve targets via the
/// jump table at `0x0100` in the other bank.
fn detect_thunks(region: &Region, source: Bank, target: Bank) -> Vec<Thunk> {
    let source_base = source.base();
    let target_base = target.base();
    let pattern = BytePattern::new("74 ?? 02 00").unwrap();

    let mut thunks = Vec::new();
    for range in region.find_bytes(&pattern) {
        let thunk_addr = range.start;
        if thunk_addr < source_base || thunk_addr >= source_base + THUNK_SEARCH_LIMIT as u32 {
            continue;
        }
        let id = region.byte_at(thunk_addr + 1).unwrap();
        let table_offset = target_base + JUMP_TABLE_BASE + 2 * id as u32;
        let Some(lo) = region.byte_at(table_offset) else {
            continue;
        };
        let Some(hi) = region.byte_at(table_offset + 1) else {
            continue;
        };
        let target_addr = target_base + ((hi as u32) << 8 | lo as u32);
        thunks.push(Thunk {
            id,
            thunk_addr,
            target_addr,
        });
    }

    thunks.sort_by_key(|thunk| (thunk.id, thunk.thunk_addr));
    thunks
}

fn mark_interrupts(region: &mut Region, bank: Bank) {
    for &(start, name) in INTERRUPT_VECTORS {
        let addr = bank.base() + start;
        region.auto_disassemble(addr).unwrap_success();
        region.set_label(addr, &format!("{name}_{bank}"));
    }
}

/// Mark jump tables, name thunk stubs/targets, and seed disassembly on both banks.
fn mark_thunks(region: &mut Region, bank: Bank, thunks: &[Thunk]) {
    for thunk in thunks {
        set_thunk_function(region, thunk.thunk_addr, thunk.id, "cross_bank_thunk");
        // eprintln!(
        //     "{bank}: cross-bank thunk 0x{id:02X} at 0x{thunk:04X} -> bank1:0x{target:04X}",
        //     id = thunk.id,
        //     thunk = thunk.thunk_addr,
        //     target = thunk.target_addr,
        // );
        region.auto_disassemble(thunk.thunk_addr).unwrap_success();
        set_target_function(region, thunk.target_addr, thunk.id, "cross_bank_target");
        region.auto_disassemble(thunk.target_addr).unwrap_success();
    }
}

fn mark_thunk_jump_table(region: &mut Region, bank: Bank, entry_count: usize) {
    region.set_label(bank.base() + JUMP_TABLE_BASE, "jump_table_base");
    if entry_count == 0 {
        return;
    }
    let table_base = bank.base() + JUMP_TABLE_BASE;
    let span = entry_count as u32 * 2;
    region.clear_equivalents(table_base, span);
    for index in 0..entry_count {
        let addr = table_base + index as u32 * 2;
        let _ = region.set_equivalent(addr, Equivalent::Data(DataType::Word, 2));
        region.set_comment(addr, &format!("jump table entry 0x{index:02X}"));
    }

    eprintln!("{bank}: cross-bank jump table 0x{JUMP_TABLE_BASE:04X}: {entry_count} entries");
}

fn set_thunk_function(region: &mut Region, addr: u32, id: u8, prefix: &str) {
    region.set_function(Function {
        addr: PhysicalAddr {
            space: AddressSpace::Code,
            offset: addr,
        },
        name: format!("{prefix}_{id:02x}"),
        signature: None,
        length: 0,
        noreturn: false,
    });
}

fn set_target_function(region: &mut Region, addr: u32, id: u8, prefix: &str) {
    set_thunk_function(region, addr, id, prefix);
}

fn apply_heuristic(label: &str, region: &mut Region, roots: Vec<(u32, String)>) -> usize {
    let mut new_roots = 0usize;
    for (addr, reason) in roots {
        match region.get_equivalent(addr) {
            EquivalentAt::Defined { start, range } => {
                if start != addr {
                    eprintln!(
                        "  WARNING: overlapped heuristic address {} -> 0x{:04X}",
                        fmt_addr(start),
                        range.end
                    );
                }
                continue;
            }
            EquivalentAt::Undefined(_) => {
                eprintln!("{}: heuristic {reason}", fmt_addr(addr));
                for error in region.auto_disassemble(addr).errors {
                    eprintln!("  ERROR: {error:?}");
                }
                new_roots += 1;
                region.set_comment(addr, &format!("{label} ({})", fmt_addr(addr)));
            }
        }
    }
    if new_roots > 0 {
        eprintln!("heuristic {label}: {new_roots} new roots");
    }
    new_roots
}

/// `PUSH DPL/DPH`, `PUSH DPL/DPH`, `MOV DPTR, #imm` (`C0 82/83 C0 82/83 90`).
fn push_dpx_mov_dptr_roots(region: &Region) -> Vec<(u32, String)> {
    const PATTERNS: &[&str] = &[
        "c0 82 c0 82 90",
        "c0 82 c0 83 90",
        "c0 83 c0 82 90",
        "c0 83 c0 83 90",
    ];
    let reason = "PUSH DPTR/MOV DPTR, #imm".to_string();
    pattern_roots(region, PATTERNS, reason)
}

/// `MOV DPTR, #0x7fxx` followed by `MOVX A, @DPTR` (`90 7F xx E0`).
fn mov_dptr_7fxx_movx_roots(region: &Region) -> Vec<(u32, String)> {
    pattern_roots(
        region,
        &["90 7f ?? e0"],
        "MOV DPTR, #0x7fxx, MOVX A, @DPTR".to_string(),
    )
}

/// Repeated `MOV DPTR` / `MOVX` sequences.
fn mov_dptr_2x_roots(region: &Region) -> Vec<(u32, String)> {
    pattern_roots(
        region,
        &[
            "90 ?? ?? f0 90 ?? ?? f0",
            "90 ?? ?? e0 90 ?? ?? e0",
            "c0 e0 90 ?? ?? e0",
        ],
        "MOV DPTR 2x".to_string(),
    )
}

fn pattern_roots(region: &Region, patterns: &[&str], reason: String) -> Vec<(u32, String)> {
    let mut roots = Vec::new();
    for pat in patterns {
        let pattern = BytePattern::new(pat).unwrap();
        for range in region.find_bytes(&pattern) {
            roots.push((range.start, reason.clone()));
        }
    }
    roots.sort_by_key(|(addr, _)| *addr);
    roots
}

/// Jump table dispatch functions (`12 ?? ?? e4 93 fa …`). Callers load DPTR then
/// `LCALL`/`LJMP` to the function; the immediate encodes the jump table address.
fn detect_jump_tables(
    region: &Region,
    bank: Bank,
    additional_rom_jump_tables: &[(&Region, &JumpTableStaticAnalysisInfo)],
) -> JumpTableStaticAnalysisInfo {
    let bank_base = bank.base();
    let bank_end = bank_base + BANK_SIZE as u32;

    // TODO: We should also use the sparse jump table even though there's only one jump table...
    let pattern = BytePattern::new("12 ?? ?? e4 93 fa a3 e4 93 f5 83 8a 82 e4 73").unwrap();
    let mut info = JumpTableStaticAnalysisInfo::default();

    for range in region.find_bytes_in(&pattern, bank_base..bank_end) {
        let local_start = range.start - bank_base;
        let hi = (local_start >> 8) as u8;
        let lo = local_start as u8;
        let mut tables = Vec::new();
        let mut found_tables = HashSet::new();

        for opcode in ["12", "02"] {
            let caller_pattern =
                BytePattern::new(&format!("90 ?? ?? {opcode} {hi:02X} {lo:02X}")).unwrap();
            for caller in region.find_bytes_in(&caller_pattern, bank_base..bank_end) {
                let hi = region.byte_at(caller.start + 1).unwrap();
                let lo = region.byte_at(caller.start + 2).unwrap();
                let table_address =
                    Bank::for_addr(caller.start).base() + ((hi as u32) << 8 | lo as u32);
                if found_tables.insert(table_address) {
                    tables.push(JumpTableInfo {
                        base: table_address,
                        range: None,
                    });
                }
            }
        }

        // TODO: Jump tables should also be limited to 1) the first address that
        // self-intersects or the address of another table.
        for (addl_region, addl_table) in additional_rom_jump_tables {
            for function in &addl_table.functions {
                if tables.len() == function.tables.len() {
                    for (table, addl_table) in tables.iter_mut().zip(function.tables.iter()) {
                        // Heuristically align tables to determine length
                        let mut addr1 = table.base;
                        let mut addr2 = addl_table.base;

                        let mut first_zero = false;
                        for i in 0..256 {
                            let a = region.read_u16_le(addr1).unwrap();
                            let b = addl_region.read_u16_le(addr2).unwrap();
                            if a == b {
                                if i == 0 {
                                    first_zero = true;
                                } else {
                                    let range = (first_zero as u32, i as u32);
                                    if let Some(existing_range) = table.range {
                                        if existing_range != range {
                                            eprintln!(
                                                "WARNING: jump table {table:04X}'s length was inconsistent across multiple differential analyses",
                                                table = table.base
                                            );
                                        }
                                    } else {
                                        table.range = Some((first_zero as u32, i as u32));
                                    }
                                    break;
                                }
                            }

                            addr1 += 2;
                            addr2 += 2;

                            // eprintln!("{addr1:04X} {a:04X} -> {addr2:04X} {b:04X} Δ{}", (a as isize - b as isize));
                        }
                    }
                }
            }
        }

        info.functions.push(JumpTableFunctionInfo {
            addr: range.start,
            length: range.end - range.start,
            tables,
        });
    }

    info
}

fn mark_jump_tables(region: &mut Region, bank: Bank, info: &JumpTableStaticAnalysisInfo) {
    for func in &info.functions {
        eprintln!(
            "{bank}: jump table function: {} - 0x{:04X}",
            fmt_addr(func.addr),
            func.addr + func.length,
        );
        region.set_function(Function {
            addr: PhysicalAddr {
                space: AddressSpace::Code,
                offset: func.addr,
            },
            name: format!("jump_table_function_{:05X}", func.addr),
            signature: None,
            length: func.length,
            noreturn: false,
        });

        for table in &func.tables {
            let bank = Bank::for_addr(table.base);
            eprintln!("{bank}: jump table: {}", fmt_addr(table.base));
            region.set_label(table.base, &format!("jump_table_{:04X}", table.base));

            if let Some(range) = table.range {
                for i in range.0..range.1 {
                    let addr = table.base + i as u32 * 2;
                    if region.has_equivalent(addr) {
                        eprintln!(
                            "WARNING: jump table {table:04X} entry 0x{i:02X} was already defined",
                            table = table.base,
                            i = i
                        );
                    } else {
                        region
                            .set_equivalent(addr, Equivalent::Data(DataType::Word, 2))
                            .unwrap();
                        let ptr = region.read_u16_le(addr).unwrap() as AddressValue + bank.base();
                        region.auto_disassemble(ptr).unwrap_success();
                    }
                    region.set_comment(addr, &format!("jump table entry 0x{i:02X}"));
                }
            }
        }
    }
}
