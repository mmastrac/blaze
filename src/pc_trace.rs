//! Linear code-coverage map sized to the ROM image: one byte per ROM byte offset
//! (`0` = not executed as code, `1` = executed). Multi-byte instructions mark every
//! opcode and operand byte, not only the first.

use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub struct PcTrace {
    pub bytes: Vec<u8>,
    path: PathBuf,
    last_flush: Instant,
}

impl PcTrace {
    pub fn new(path: PathBuf, rom_len: usize) -> io::Result<Self> {
        let bytes = load_or_init(&path, rom_len)?;
        Ok(Self {
            bytes,
            path,
            last_flush: Instant::now(),
        })
    }

    /// Mark `byte_count` bytes starting at `start` (linear ROM offset) as executed (`1`).
    pub fn mark_range(&mut self, start: u32, byte_count: usize) {
        if byte_count == 0 {
            return;
        }
        let start = start as usize;
        if start >= self.bytes.len() {
            return;
        }
        let end = start.saturating_add(byte_count).min(self.bytes.len());
        for b in &mut self.bytes[start..end] {
            *b = 1;
        }
    }

    /// Call periodically from the CPU step loop; flushes at most once per second.
    pub fn flush_if_due(&mut self) {
        if self.last_flush.elapsed() >= Duration::from_secs(1) {
            if let Err(e) = self.flush_now() {
                tracing::warn!("pc-trace flush failed: {e}");
            }
            self.last_flush = Instant::now();
        }
    }

    pub fn flush_now(&mut self) -> io::Result<()> {
        let mut f = File::create(&self.path)?;
        f.write_all(&self.bytes)?;
        f.flush()?;
        Ok(())
    }
}

impl Drop for PcTrace {
    fn drop(&mut self) {
        let _ = self.flush_now();
    }
}

fn load_or_init(path: &Path, rom_len: usize) -> io::Result<Vec<u8>> {
    let mut v = if path.exists() {
        std::fs::read(path)?
    } else {
        vec![0u8; rom_len]
    };

    if v.len() < rom_len {
        v.resize(rom_len, 0);
    } else if v.len() > rom_len {
        v.truncate(rom_len);
    }

    for b in &mut v {
        *b = if *b == 0 { 0 } else { 1 };
    }

    Ok(v)
}
