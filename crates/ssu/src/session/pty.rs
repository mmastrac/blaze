use std::num::NonZeroU16;
use std::thread;
use std::{fs::File, io, os::fd::OwnedFd};

use pty_process::blocking::Command;

use crate::session::io_session::{IoSession, IoSessionReadWrite};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecPtyConfig {
    pub cmd: String,
    pub rows: NonZeroU16,
    pub cols: NonZeroU16,
}

impl ExecPtyConfig {
    fn start(self) -> io::Result<IoSessionReadWrite> {
        let (pty, pts) = pty_process::blocking::open()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        pty.resize(pty_process::Size::new(self.rows.into(), self.cols.into()))
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        // Spawn command via shell
        let _child = Command::new("/bin/sh")
            .arg("-c")
            .arg(&self.cmd)
            .spawn(pts)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        let pty = File::from(OwnedFd::from(pty));
        let pty_read: File = pty.try_clone()?;

        Ok(IoSessionReadWrite::new(pty_read, pty))
    }
}

impl IoSession for ExecPtyConfig {
    fn start(self, ready: impl FnOnce(std::io::Result<IoSessionReadWrite>) + Send + 'static) {
        thread::spawn(move || {
            let result = self.start();
            ready(result);
        });
    }
}
