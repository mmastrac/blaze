use std::process::Stdio;
use std::{io, thread};

use crate::session::io_session::{IoSession, IoSessionReadWrite};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecConfig {
    pub command: String,
}

impl ExecConfig {
    pub fn start(self) -> io::Result<IoSessionReadWrite> {
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

        Ok(IoSessionReadWrite::new(stdout, stdin))
    }
}

impl IoSession for ExecConfig {
    fn start(self, ready: impl FnOnce(std::io::Result<IoSessionReadWrite>) + Send + 'static) {
        thread::spawn(move || {
            let result = self.start();
            ready(result);
        });
    }
}
