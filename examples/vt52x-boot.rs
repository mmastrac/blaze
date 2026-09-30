//! Boot the VT520 ROM headless and report how far it gets.
//!
//! `cargo run --release --example vt52x-boot -- [instructions] [log level]`
use std::collections::HashMap;
use std::fs;

use blaze_vt::machine::vt52x::System;
use i8051::Cpu;
use i8051::sfr::SFR_B;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let count: usize = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(20_000_000);
    let level: tracing::Level = args
        .get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(tracing::Level::INFO);
    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_writer(std::io::stderr)
        .init();

    // ROMFILE=path boots another ROM (default: the VT520's).
    let rom = fs::read(std::env::var("ROMFILE").unwrap_or_else(|_| {
        concat!(env!("CARGO_MANIFEST_DIR"), "/roms/vt520/23-010ED-00.bin").to_string()
    }))
    .unwrap();
    // NVR=path loads (and keeps saving to) a 2KB NVR image.
    let nvr = std::env::var("NVR").ok().map(std::path::PathBuf::from);
    // COMM1=<session config> links comm1 to a host session (e.g. "loopback").
    let comm1 = std::env::var("COMM1")
        .ok()
        .map(|v| <ssu::session::SessionConfig as std::str::FromStr>::from_str(&v).unwrap())
        .map(|config| config.start_unsend().unwrap());
    let script = std::env::var("SCRIPT")
        .ok()
        .map(|path| blaze_vt::machine::generic::script::Script::load(path.as_ref()).unwrap());
    let comm1 = match &script {
        Some(script) => Some(script.comm_session(0, comm1)),
        None => comm1,
    };
    let comm2 = script.as_ref().map(|script| script.comm_session(1, None));
    let mut system = System::new(rom, nvr.as_deref(), comm1, comm2).unwrap();
    if let Some(script) = script {
        system.script = script;
    }
    // FACTORY=1 fits the factory test fixture: no keyboard, and a loopback
    // plug on the keyboard port.
    if std::env::var("FACTORY").is_ok() {
        system.memory.regs.factory_plug.fitted = true;
        system.memory.regs.kbd.present = false;
    }
    if let Some(mask) = std::env::var("PAGEMASK")
        .ok()
        .and_then(|v| u8::from_str_radix(&v, 16).ok())
    {
        system.memory.page_mask = mask;
    }
    if let Some(n) = std::env::var("PAGES")
        .ok()
        .and_then(|v| u8::from_str_radix(&v, 16).ok())
    {
        system.memory.pages_fitted = n;
    }
    if let Some((a, b)) = std::env::var("DWATCH").ok().and_then(|v| {
        let (a, b) = v.split_once('-')?;
        Some((
            usize::from_str_radix(a, 16).ok()?,
            usize::from_str_radix(b, 16).ok()?,
        ))
    }) {
        system.memory.dram_watch = Some((a, b));
    }
    if let Some((a, b)) = std::env::var("LWATCH").ok().and_then(|v| {
        let (a, b) = v.split_once('-')?;
        Some((
            usize::from_str_radix(a, 16).ok()?,
            usize::from_str_radix(b, 16).ok()?,
        ))
    }) {
        system.memory.low_watch = Some((a, b));
    }
    if let Some(p) = std::env::var("COMMONHI")
        .ok()
        .and_then(|v| u8::from_str_radix(&v, 16).ok())
    {
        system.memory.common_high_page = Some(p);
    }
    if std::env::var("WINDOW_MISMATCH").is_ok() {
        system.memory.window_writer = Some(vec![0xFF; 0x8000]);
    }
    if let Some(v) = std::env::var("MODEM")
        .ok()
        .and_then(|v| u8::from_str_radix(&v, 16).ok())
    {
        system.memory.regs.modem_status = v;
    }
    // PCKBD=1 makes the keyboard answer like a PC keyboard instead of a DEC
    // LK keyboard.
    if std::env::var("PCKBD").is_ok() {
        system.memory.regs.kbd.pc_keyboard = true;
    }
    // NOPIPE=1 turns off the one-behind XDATA read latch.
    if std::env::var("NOPIPE").is_ok() {
        system.memory.pipelined_reads = false;
    }
    // FORCE=reg=val,... makes reads of live register 0x7Freg return val.
    for item in std::env::var("FORCE")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
    {
        let (r, v) = item.split_once('=').unwrap();
        system.memory.regs.forced_reads.push((
            u8::from_str_radix(r, 16).unwrap(),
            u8::from_str_radix(v, 16).unwrap(),
        ));
    }
    // PINREADS=1 prints which PCs read P1/P3 pins.
    let pin_reads = std::env::var("PINREADS").is_ok();
    if pin_reads {
        system.log_pin_reads();
    }
    let mut cpu = Cpu::new();

    // PC histogram over the last tenth of the run, to find where it settles.
    let window = count / 10;
    let mut hist: HashMap<u32, usize> = HashMap::new();
    // PCSET_FROM=n with PCSET=path writes every PC executed from instruction n.
    let mut pcset = std::collections::BTreeSet::new();
    // TRACE=from:to:path writes each PC the first time it runs in [from, to), in order.
    let trace: Option<(usize, usize, String)> = std::env::var("TRACE").ok().map(|v| {
        let mut it = v.splitn(3, ':');
        let from = it.next().unwrap().parse().unwrap();
        let to = it.next().unwrap().parse().unwrap();
        (from, to, it.next().unwrap().to_string())
    });
    // REGLOG=from:to prints the register writes made in [from, to), grouped by
    // (register, value, pc), skipping the blitter and keyboard registers.
    let reglog: Option<(usize, usize)> = std::env::var("REGLOG").ok().map(|v| {
        let (a, b) = v.split_once(':').unwrap();
        (a.parse().unwrap(), b.parse().unwrap())
    });
    // READLOG=from:to prints register reads in [from, to), grouped by
    // (register, value, pc), in first-seen order.
    let readlog: Option<(usize, usize)> = std::env::var("READLOG").ok().map(|v| {
        let (a, b) = v.split_once(':').unwrap();
        (a.parse().unwrap(), b.parse().unwrap())
    });
    let mut trace_seen = std::collections::HashSet::new();
    let mut trace_out = String::new();
    let report_every = (count / 20).max(1);
    // WATCH=hex,hex prints the recent PC trail whenever one of these PCs runs.
    let watch: Vec<u32> = std::env::var("WATCH")
        .unwrap_or_default()
        .split(',')
        .filter_map(|s| u32::from_str_radix(s.trim(), 16).ok())
        .collect();
    let mut trail = std::collections::VecDeque::with_capacity(40);
    let mut last_pc = 0;
    // SPMIN=hex: once a watched PC runs, report the first PC where SP drops below this.
    let sp_min = std::env::var("SPMIN")
        .ok()
        .and_then(|s| u8::from_str_radix(&s, 16).ok());
    let mut sp_armed = false;
    // SAVE_NVR=path: once the firmware reaches its main loop (bank 0 0x2410),
    // call the firmware's own "save all settings" routine (bank 0 0x8C22) so it
    // writes its defaults to the NVR with valid checksums, then save the image.
    let save_nvr = std::env::var("SAVE_NVR").ok();
    // SAVE_NVR_AT / SAVE_NVR_CALL override those two addresses (hex) for other
    // ROMs (VT525: 2190 / 8BF8).
    let hex_env = |name: &str, default: u32| {
        std::env::var(name)
            .ok()
            .and_then(|v| u32::from_str_radix(&v, 16).ok())
            .unwrap_or(default)
    };
    let save_nvr_at = hex_env("SAVE_NVR_AT", 0x2410);
    let save_nvr_call = hex_env("SAVE_NVR_CALL", 0x8C22) as u16;
    let mut save_called = false;
    // KEYS=at:code,... presses scan-code-set-3 key `code` (hex) at instruction
    // `at` and releases it 60k instructions later.
    let mut keys: Vec<(usize, Vec<u8>)> = vec![];
    // HOST=at:text feeds `text` into comm1's receive buffer at instruction
    // `at` (\e = ESC, \r, \n, \a = BEL). HOST2 does the same for comm2.
    // Several chunks can be given as at:text@@at:text.
    let host: Vec<(usize, usize, Vec<u8>)> = ["HOST", "HOST2"]
        .iter()
        .enumerate()
        .filter_map(|(ch, name)| std::env::var(name).ok().map(|v| (ch, v)))
        .flat_map(|(ch, v)| {
            v.split("@@")
                .map(|chunk| (ch, chunk.to_string()))
                .collect::<Vec<_>>()
        })
        .filter_map(|(ch, v)| {
            let (at, text) = v.split_once(':')?;
            let text = text
                .replace("\\e", "\x1b")
                .replace("\\r", "\r")
                .replace("\\n", "\n")
                .replace("\\a", "\x07");
            Some((ch, at.parse::<usize>().ok()?, text.into_bytes()))
        })
        .collect();
    for item in std::env::var("KEYS")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
    {
        // A combination like 12+1F presses the codes in order and releases
        // them in reverse (Shift+F4).
        let (at, combo) = item.split_once(':').unwrap();
        let at: usize = at.parse().unwrap();
        let codes: Vec<u8> = combo
            .split('+')
            .map(|c| u8::from_str_radix(c, 16).unwrap())
            .collect();
        keys.push((at, codes.clone()));
        let release: Vec<u8> = codes.iter().rev().flat_map(|&c| [0xF0, c]).collect();
        keys.push((at + 60_000, release));
    }
    // BLITLOG_FROM=n prints each blit (cmd, src, dst) from instruction n.
    let blitlog_from: Option<usize> = std::env::var("BLITLOG_FROM")
        .ok()
        .and_then(|v| v.parse().ok());
    let mut last_blits = 0;
    // POKE=at:addr=val,... writes an XDATA (DRAM) byte at instruction `at`.
    let pokes: Vec<(usize, usize, u8)> = std::env::var("POKE")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|item| {
            let (at, rest) = item.split_once(':').unwrap();
            let (addr, val) = rest.split_once('=').unwrap();
            (
                at.parse().unwrap(),
                usize::from_str_radix(addr, 16).unwrap(),
                u8::from_str_radix(val, 16).unwrap(),
            )
        })
        .collect();
    let pcset_from: Option<usize> = std::env::var("PCSET_FROM")
        .ok()
        .and_then(|v| v.parse().ok());
    let reglog_all = std::env::var("REGLOG_ALL").is_ok();
    // HOSTFILE=at:path feeds a file into comm1 from instruction `at`, one byte
    // at a time while the receive ring holds fewer than 256 bytes.
    let mut hostfile: Option<(usize, std::collections::VecDeque<u8>)> =
        std::env::var("HOSTFILE").ok().map(|v| {
            let (at, path) = v.split_once(':').unwrap();
            (at.parse().unwrap(), fs::read(path).unwrap().into())
        });
    // BLITSTACK=n prints SP, the P1 bank and the stack bytes at the first blit
    // at or after instruction n.
    let mut blitstack_from: Option<usize> =
        std::env::var("BLITSTACK").ok().and_then(|v| v.parse().ok());
    for i in 0..count {
        system.step(&mut cpu);
        if let Some(from) = blitstack_from {
            if i >= from && system.memory.regs.blit_count != last_blits {
                let sp = cpu.sfr(i8051::sfr::SFR_SP, &system) as usize;
                let bytes: Vec<String> = (0x60.max(sp.saturating_sub(40))..=sp)
                    .map(|a| format!("{a:02X}:{:02X}", cpu.internal_ram[a]))
                    .collect();
                println!(
                    "blitstack@{i} cmd={:02X} pc={:05X} SP={sp:02X} {}",
                    system.memory.regs.regs[0xC9],
                    cpu.pc_ext(&system),
                    bytes.join(" ")
                );
                blitstack_from = None;
            }
        }
        if let Some(from) = blitlog_from {
            if i >= from && system.memory.regs.blit_count != last_blits {
                let r = &system.memory.regs.regs;
                println!(
                    "blit@{i} cmd={:02X} src={:02X}{:02X}{:02X} dst={:02X}{:02X}{:02X} cnt={:02X}{:02X}{:02X} CA-CE={:02X} {:02X} {:02X} {:02X} {:02X} CF={:02X}",
                    r[0xC9],
                    r[0xC2],
                    r[0xC1],
                    r[0xC0],
                    r[0xC5],
                    r[0xC4],
                    r[0xC3],
                    r[0xC8],
                    r[0xC7],
                    r[0xC6],
                    r[0xCA],
                    r[0xCB],
                    r[0xCC],
                    r[0xCD],
                    r[0xCE],
                    r[0xCF]
                );
            }
        }
        last_blits = system.memory.regs.blit_count;
        for &(at, addr, val) in &pokes {
            if at == i {
                system.memory.dram[blaze_vt::machine::vt52x::memory::LOW_XDATA_BASE + addr] = val;
            }
        }
        if let Some((at, bytes)) = &mut hostfile {
            if i >= *at && i % 64 == 0 && system.memory.comm[0].rx_pending < 0x100 {
                if let Some(b) = bytes.pop_front() {
                    system.memory.comm_receive(0, b);
                }
            }
        }
        for (ch, at, text) in &host {
            if *at == i {
                for &b in text {
                    system.memory.comm_receive(*ch, b);
                }
            }
        }
        for (at, bytes) in &keys {
            if *at == i {
                system.memory.regs.kbd.send(bytes);
            }
        }
        let pc = cpu.pc_ext(&system);
        if save_nvr.is_some() && !save_called && pc == save_nvr_at {
            eprintln!("[{i:>10}] calling save-all settings ({save_nvr_call:04X})");
            cpu.push_stack16(cpu.pc);
            cpu.pc = save_nvr_call;
            save_called = true;
        }
        // Keep only jumps (non-sequential PCs) in the trail.
        if pc.wrapping_sub(last_pc) > 3 {
            if trail.len() == 40 {
                trail.pop_front();
            }
            trail.push_back(last_pc);
        }
        last_pc = pc;
        if sp_armed {
            let sp = cpu.sfr(0x81, &system);
            if Some(sp) < sp_min {
                let t: Vec<String> = trail
                    .iter()
                    .skip(trail.len().saturating_sub(30))
                    .map(|p| format!("{p:05X}"))
                    .collect();
                eprintln!(
                    "[{i:>10}] SP dropped to {sp:02X} at {pc:05X} (prev {:05X}) after {}",
                    trail.back().copied().unwrap_or(0),
                    t.join(" ")
                );
                sp_armed = false;
            }
        }
        if watch.contains(&pc) {
            sp_armed = sp_min.is_some();
            // Call stack: 16-bit return addresses from SP down to 0x80.
            let sp = cpu.sfr(0x81, &system) as usize;
            let mut frames = vec![];
            let mut k = sp;
            while k > 0x80 {
                frames.push(format!(
                    "{:02X}{:02X}",
                    cpu.internal_ram[k],
                    cpu.internal_ram[k - 1]
                ));
                k -= 2;
            }
            eprintln!("[{i:>10}] stack at {pc:05X}: {}", frames.join(" "));
            let t: Vec<String> = trail
                .iter()
                .skip(trail.len().saturating_sub(30))
                .map(|p| format!("{p:05X}"))
                .collect();
            let sp = cpu.sfr(0x81, &system);
            let top = cpu.internal_ram[sp as usize] as u16
                | (cpu.internal_ram[sp.wrapping_sub(1) as usize] as u16) << 8;
            let rb = (cpu.sfr(0xD0, &system) & 0x18) as usize;
            let r: Vec<String> = (0..8)
                .map(|k| format!("{:02X}", cpu.internal_ram[rb + k]))
                .collect();
            let p2 = cpu.sfr(0xA0, &system);
            let r1 = cpu.internal_ram[rb + 1];
            eprintln!(
                "[{i:>10}] watch {pc:05X} A={:02X} B={:02X} DPTR={:02X}{:02X} R0-7={} P2={p2:02X} @R1 -> {} (3B={:02X} 3F={:02X} 36={:02X})",
                cpu.sfr(0xE0, &system),
                cpu.sfr(0xF0, &system),
                cpu.sfr(0x83, &system),
                cpu.sfr(0x82, &system),
                r.join(" "),
                system.memory.describe((p2 as u16) << 8 | r1 as u16),
                system.memory.regs.regs[0xBB],
                system.memory.regs.regs[0xBF],
                system.memory.regs.regs[0xB6]
            );
            eprintln!(
                "[{i:>10}] watch {pc:05X} sp={sp:02X} top={top:04X} x7410={:02X} {:02X} after {}",
                system.memory.dram_byte_at_xdata(0x7410),
                system.memory.dram_byte_at_xdata(0x7411),
                t.join(" ")
            );
        }
        if let Some((from, to)) = readlog {
            if i == from {
                *system.memory.read_log.borrow_mut() = Some(vec![]);
            }
            if i == to {
                let log = system
                    .memory
                    .read_log
                    .borrow_mut()
                    .take()
                    .unwrap_or_default();
                let mut seen: Vec<((u8, u8, u32), usize)> = vec![];
                for w in log {
                    match seen.iter_mut().find(|(k, _)| *k == w) {
                        Some((_, n)) => *n += 1,
                        None => seen.push((w, 1)),
                    }
                }
                for ((reg, value, pc), n) in seen {
                    println!("read 7F{reg:02X} = {value:02X} @ {pc:05X} x{n}");
                }
            }
        }
        if let Some((from, to)) = reglog {
            if i == from {
                system.memory.regs.write_log = Some(vec![]);
            }
            if i == to {
                let log = system.memory.regs.write_log.take().unwrap_or_default();
                let mut seen: Vec<((u8, u8, u32), usize)> = vec![];
                for w in log {
                    if !reglog_all && ((0xC0..=0xCF).contains(&w.0) || w.0 >= 0xF8) {
                        continue;
                    }
                    match seen.iter_mut().find(|(k, _)| *k == w) {
                        Some((_, n)) => *n += 1,
                        None => seen.push((w, 1)),
                    }
                }
                for ((reg, value, pc), n) in seen {
                    println!("reg 7F{reg:02X} = {value:02X} @ {pc:05X} x{n}");
                }
            }
        }
        if let Some((from, to, _)) = &trace {
            if (*from..*to).contains(&i) && trace_seen.insert(pc) {
                trace_out.push_str(&format!("{i} {pc:05X}\n"));
            }
        }
        if pcset_from.is_some_and(|from| i >= from) {
            pcset.insert(pc);
        }
        if i >= count - window {
            *hist.entry(pc).or_default() += 1;
        }
        if i % report_every == 0 {
            let ram = &cpu.internal_ram;
            eprintln!(
                "[{i:>10}] pc={pc:05X} frames={} blits={} 77={:02X} 58={:02X} 59={:02X} leds={:02X}",
                system.frame_count,
                system.memory.regs.blit_count,
                ram[0x77],
                ram[0x58],
                ram[0x59],
                system.keyboard_leds()
            );
        }
    }

    if let Ok(path) = std::env::var("PCSET") {
        let text: Vec<String> = pcset.iter().map(|p| format!("{p:05X}")).collect();
        fs::write(path, text.join("\n")).unwrap();
    }
    let mut top: Vec<_> = hist.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1));
    println!("Hot PCs in the last {window} instructions:");
    for (pc, n) in top.iter().take(12) {
        println!("  {pc:05X} {n}");
    }
    let ram = &cpu.internal_ram;
    println!(
        "B={:02X} 0x74={:02X} 0x77={:02X} 0x58={:02X} 0x59={:02X} 0x60={:02X} leds={:02X}",
        cpu.sfr(SFR_B, &system),
        ram[0x74],
        ram[0x77],
        ram[0x58],
        ram[0x59],
        ram[0x60],
        system.keyboard_leds()
    );
    // DUMP=path writes low XDATA (32KB) followed by paged RAM for offline study.
    if let Ok(path) = std::env::var("DUMP") {
        let mut all = (0..0x8000u16)
            .map(|a| system.memory.dram_byte_at_xdata(a))
            .collect::<Vec<u8>>();
        all.extend_from_slice(&system.memory.dram);
        all.extend_from_slice(&system.memory.regs.regs);
        all.extend_from_slice(&cpu.internal_ram);
        fs::write(&path, all).unwrap();
        println!("Wrote memory dump to {path}");
    }
    println!(
        "RAM probe E0-E7: {:02X?}  0x22={:08b} 0x27={:08b}",
        &ram[0xE0..0xE8],
        ram[0x22],
        ram[0x27]
    );
    // PALETTE=1 prints the palette DAC's first 32 entries (6-bit R, G, B).
    if std::env::var("PALETTE").is_ok() {
        let ramdac = &system.memory.regs.ramdac;
        for (i, rgb) in ramdac.palette.iter().enumerate().take(32) {
            println!(
                "  palette {i:02X}: {:02X} {:02X} {:02X}",
                rgb[0], rgb[1], rgb[2]
            );
        }
        println!("  palette mask {:02X}", ramdac.mask);
    }
    println!(
        "Unhandled blit commands: {:?}",
        system.memory.regs.unknown_blits
    );
    println!(
        "IDATA 4B/4C = {:02X}{:02X}, 0x7F31={:02X}",
        ram[0x4C], ram[0x4B], system.memory.regs.regs[0x31]
    );
    println!("Line table 0x7E00:");
    for row in (0..0x100).step_by(16) {
        let bytes: Vec<String> = (0..16)
            .map(|k| {
                format!(
                    "{:02X}",
                    system.memory.dram_byte_at_xdata((0x7E00 + row + k) as u16)
                )
            })
            .collect();
        println!("  {:04X}: {}", 0x7E00 + row, bytes.join(" "));
    }
    // FB=y0:y1[:x0:x1] prints rendered display lines y0..y1, cells x0..x1
    // (default 0..16), 10 pixels each.
    if let Ok(spec) = std::env::var("FB") {
        use blaze_vt::machine::generic::display::{Display, FRAME_HEIGHT, FRAME_WIDTH};
        let v: Vec<usize> = spec.split(':').map(|n| n.parse().unwrap()).collect();
        let (y0, y1) = (v[0], v[1]);
        let (x0, x1) = (
            v.get(2).copied().unwrap_or(0),
            v.get(3).copied().unwrap_or(16),
        );
        let mut frame = vec![0u8; FRAME_WIDTH * FRAME_HEIGHT * 4];
        system.render_framebuffer(&mut frame);
        let fg = blaze_vt::machine::generic::color::DEFAULT_COLOR.foreground;
        for y in y0..y1.min(FRAME_HEIGHT) {
            let px: String = (x0 * 10..(x1 * 10).min(FRAME_WIDTH))
                .map(|x| {
                    if frame[(y * FRAME_WIDTH + x) * 4..][..3] == [fg.0, fg.1, fg.2] {
                        '#'
                    } else {
                        '.'
                    }
                })
                .collect();
            println!("  {y:3} {px}");
        }
    }

    if let Some(path) = &save_nvr {
        fs::write(path, system.nvr()).unwrap();
        println!("Wrote NVR image to {path}");
    }
    if let Some((_, _, path)) = &trace {
        fs::write(path, &trace_out).unwrap();
    }
    if pin_reads {
        for ((addr, pc), n) in system.pin_reads() {
            let port = match addr {
                0x90 => "P1",
                0xB0 => "P3",
                _ => "P2",
            };
            println!("pin read {port} @ {pc:05X} x{n}");
        }
    }
    // IPEEK=1 prints internal RAM at the end of the run.
    if std::env::var("IPEEK").is_ok() {
        for row in (0..0x100).step_by(16) {
            let bytes: Vec<String> = (row..row + 16)
                .map(|k| format!("{:02X}", cpu.internal_ram[k]))
                .collect();
            println!("idata {row:02X}: {}", bytes.join(" "));
        }
    }
    // PEEK=addr[+len],... prints XDATA bytes at the end of the run.
    for item in std::env::var("PEEK")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
    {
        let (addr, len) = item.split_once('+').unwrap_or((item, "10"));
        let (addr, len) = (
            u16::from_str_radix(addr, 16).unwrap(),
            usize::from_str_radix(len, 16).unwrap(),
        );
        for row in (0..len).step_by(16) {
            let bytes: Vec<String> = (row..(row + 16).min(len))
                .map(|k| {
                    format!(
                        "{:02X}",
                        system
                            .memory
                            .dram_byte_at_xdata(addr.wrapping_add(k as u16))
                    )
                })
                .collect();
            println!(
                "peek {:04X}: {}",
                addr.wrapping_add(row as u16),
                bytes.join(" ")
            );
        }
    }
    println!(
        "0x91A9 (NVR bad) = {:02X}",
        system.memory.dram_byte_at_xdata(0x91A9)
    );
    println!(
        "NVR writes: {}",
        system.nvr().iter().filter(|&&b| b != 0xFF).count()
    );
    // RAWGRID=1 prints the stored rows in framebuffer order.
    if std::env::var("RAWGRID").is_ok() {
        for row in 0..blaze_vt::machine::vt52x::memory::TEXT_ROWS {
            let text: String = system
                .memory
                .screen_row(row)
                .iter()
                .filter(|c| !c.continuation)
                .map(|c| match c.ch {
                    0x20..=0x7E => c.ch as char,
                    _ => '·',
                })
                .collect();
            if !text.trim().is_empty() {
                println!("grid {row:2}|{}", text.trim_end());
            }
        }
    }
    // RAWROW=n prints the raw character codes on text row n.
    if let Some(row) = std::env::var("RAWROW")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
    {
        let codes: Vec<String> = system.memory.screen_row(row)[..48]
            .iter()
            .map(|c| {
                format!(
                    "{:02X}/{:02X}{}",
                    c.ch,
                    c.attr,
                    if c.continuation { "+" } else { "" }
                )
            })
            .collect();
        println!("row {row} codes: {}", codes.join(" "));
    }
    if system.memory.window_writer.is_some() {
        let mm = system.memory.window_mismatch.borrow();
        let mut v: Vec<_> = mm.iter().collect();
        v.sort_by(|a, b| b.1.cmp(a.1));
        println!(
            "Window reads through a different page than the last writer (writer, reader, 4K region: count):"
        );
        for ((w, r, reg), n) in v.iter().take(30) {
            println!(
                "  writer {w:02X} reader {r:02X} region {:04X}: {n}",
                0x8000 + (*reg as u32) * 0x1000
            );
        }
    }
    for (ch, chan) in system.memory.comm.iter().enumerate() {
        if !chan.tx_out.is_empty() {
            let bytes: Vec<String> = chan.tx_out.iter().map(|b| format!("{b:02X}")).collect();
            println!("comm{} sent: {}", ch + 1, bytes.join(" "));
        }
    }
    // PNG=path writes the rendered framebuffer.
    if let Ok(path) = std::env::var("PNG") {
        blaze_vt::machine::generic::display::save_png(&system, std::path::Path::new(&path))
            .unwrap();
        println!("Wrote framebuffer to {path}");
    }
    println!("Screen (from character blits):");
    let lines = blaze_vt::machine::generic::display::text_lines(&system);
    let last = lines.iter().rposition(|l| !l.is_empty()).unwrap_or(0);
    let rows = blaze_vt::machine::generic::display::text_grid(&system);
    for (row, line) in lines.iter().take(last + 1).enumerate() {
        println!("  {row:2}|{line}");
        // Mark attributes under the row: R reverse, B bold, U underline,
        // K blink (first match wins).
        use blaze_vt::machine::generic::display::TextAttr;
        let marks: String = rows[row]
            .iter()
            .map(|&(_, a)| match a {
                a if a.contains(TextAttr::REVERSE) => 'R',
                a if a.contains(TextAttr::BOLD) => 'B',
                a if a.contains(TextAttr::UNDERLINE) => 'U',
                a if a.contains(TextAttr::BLINK) => 'K',
                _ => ' ',
            })
            .collect();
        if !marks.trim().is_empty() {
            println!("    :{}", marks.trim_end());
        }
    }
}
