use std::num::NonZeroU16;
use std::thread;
use std::{fs::File, io, os::fd::OwnedFd, sync::mpsc};

use pty_process::blocking::Command;

use crate::session::IoSessionEndpoint;
use crate::session::io::IoSession;

pub struct PtySession {
    command: String,
    cols: NonZeroU16,
    rows: NonZeroU16,
}

impl PtySession {
    pub fn new(command: String, cols: NonZeroU16, rows: NonZeroU16) -> Self {
        PtySession {
            command,
            cols,
            rows,
        }
    }

    fn start(self, rx: mpsc::SyncSender<u8>, tx: mpsc::Receiver<u8>) -> io::Result<IoSession> {
        let (pty, pts) = pty_process::blocking::open()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        pty.resize(pty_process::Size::new(self.rows.into(), self.cols.into()))
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        // Spawn command via shell
        let _child = Command::new("/bin/sh")
            .arg("-c")
            .arg(&self.command)
            .spawn(pts)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        let pty = File::from(OwnedFd::from(pty));
        let pty_read: File = pty.try_clone()?;

        Ok(IoSession::new(pty_read, pty, rx, tx))
    }
}

impl IoSessionEndpoint for PtySession {
    fn start(
        self,
        rx: mpsc::SyncSender<u8>,
        tx: mpsc::Receiver<u8>,
        ready: impl FnOnce(std::io::Result<IoSession>) + Send + 'static,
    ) {
        thread::spawn(move || {
            let result = self.start(rx, tx);
            ready(result);
        });
    }
}
