use std::process::Stdio;
use std::sync::mpsc;
use std::{io, thread};

use crate::session::{IoSessionEndpoint, io::IoSession};

pub struct ExecSession {
    command: String,
}

impl ExecSession {
    pub fn new(command: String) -> Self {
        ExecSession { command }
    }

    pub fn start(self, rx: mpsc::SyncSender<u8>, tx: mpsc::Receiver<u8>) -> io::Result<IoSession> {
        // Spawn command via shell
        let mut child = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(&self.command)
            .stderr(Stdio::null())
            .stdout(Stdio::piped())
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();

        Ok(IoSession::new(stdout, stdin, rx, tx))
    }
}

impl IoSessionEndpoint for ExecSession {
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
