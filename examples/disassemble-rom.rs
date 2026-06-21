use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use blaze_vt::machine::vt420::static_analysis::{Bank, auto_analyze, load_rom, process_heuristics};
use clap::Parser;
use i8051_disassembler::address::AddressValue;
use i8051_disassembler::render::sdas::SdasWriter;
use i8051_disassembler::{address::AddressSpace, db::Db};

const BANK_SIZE: usize = 0x1_0000;
const ROM_SIZE: usize = 2 * BANK_SIZE;

fn fmt_addr(addr: AddressValue) -> String {
    format!("0x{addr:05X}")
}

/// Disassemble an 8051 ROM image into SDAS assembly.
#[derive(Parser)]
#[command(name = "disassemble-rom")]
struct Args {
    /// Path to the ROM file
    #[arg(short, long)]
    rom: PathBuf,

    /// Additional ROM files to use for differential analysis (recommended)
    #[arg(short, long)]
    additional_rom: Vec<PathBuf>,

    /// Output directory (writes `bank0.asm` and `bank1.asm`)
    #[arg(short, long)]
    output: PathBuf,

    /// Optional ROM-sized execution trace (non-zero = byte was executed as code); seeds disassembly roots
    #[arg(long)]
    pc_trace: Option<PathBuf>,

    /// Enable verbose output
    #[arg(short, long)]
    verbose: bool,
}

fn main() -> io::Result<()> {
    let args = Args::parse();
    let rom = fs::read(&args.rom)?;
    if rom.len() != ROM_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "ROM is not 128kB",
        ));
    }
    let maybe_pc_trace = match &args.pc_trace {
        None => None,
        Some(path) => Some(load_pc_trace(path)?),
    };

    fs::create_dir_all(&args.output)?;

    let mut additional_roms = Vec::new();
    for additional_rom in args.additional_rom {
        let additional_rom = fs::read(&additional_rom)?;
        if additional_rom.len() != ROM_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Additional ROM is not 128kB",
            ));
        }
        additional_roms.push(load_rom(additional_rom, vec![])?);
    }

    let info = load_rom(rom, additional_roms)?;
    let mut db = auto_analyze(info, args.verbose)?;
    process_heuristics(&mut db, maybe_pc_trace)?;

    write_bank_asm(&db, Bank::Bank0, &args.output.join("bank0.asm"))?;
    write_bank_asm(&db, Bank::Bank1, &args.output.join("bank1.asm"))?;

    Ok(())
}

fn load_pc_trace(path: &Path) -> io::Result<Vec<u8>> {
    let mut trace = if path.exists() {
        fs::read(path)?
    } else {
        vec![0; ROM_SIZE]
    };

    if trace.len() < ROM_SIZE {
        trace.resize(ROM_SIZE, 0);
    } else if trace.len() > ROM_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "pc-trace is too large",
        ));
    }

    Ok(trace)
}

fn write_bank_asm(db: &Db, bank: Bank, output: &Path) -> io::Result<()> {
    let start = bank.base();
    let end = start + BANK_SIZE as u32;
    let mut writer = SdasWriter::default();
    writer.write(AddressSpace::Code.area_header());
    for line in db.render_range(AddressSpace::Code, start, end) {
        writer.write_line(&line);
    }
    fs::write(output, writer.into_string())?;
    eprintln!("{}: wrote {output:?}", fmt_addr(start));
    Ok(())
}
