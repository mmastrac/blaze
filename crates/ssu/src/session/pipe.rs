use std::{fs::OpenOptions, io, path::PathBuf, thread};

use crate::session::IoSessionEndpoint;
use crate::session::io::IoSessionReadWrite;

pub struct SinglePipeSession {
    path: PathBuf,
}

impl SinglePipeSession {
    pub fn new(path: PathBuf) -> Self {
        SinglePipeSession { path }
    }

    fn start(self) -> io::Result<IoSessionReadWrite> {
        let pipe_r = OpenOptions::new().read(true).write(true).open(&self.path)?;
        let pipe_w = pipe_r.try_clone()?;
        Ok(IoSessionReadWrite::new(pipe_r, pipe_w))
    }
}

impl IoSessionEndpoint for SinglePipeSession {
    fn start(self, ready: impl FnOnce(io::Result<IoSessionReadWrite>) + Send + 'static) {
        thread::spawn(move || {
            let result = self.start();
            ready(result);
        });
    }
}

pub struct DualPipeSession {
    recv: PathBuf,
    send: PathBuf,
}

impl DualPipeSession {
    pub fn new(recv: PathBuf, send: PathBuf) -> Self {
        DualPipeSession { recv, send }
    }

    fn start(self) -> io::Result<IoSessionReadWrite> {
        let pipe_r = OpenOptions::new().read(true).open(&self.recv)?;
        let pipe_w = OpenOptions::new().write(true).open(&self.send)?;
        Ok(IoSessionReadWrite::new(pipe_r, pipe_w))
    }
}

impl IoSessionEndpoint for DualPipeSession {
    fn start(self, ready: impl FnOnce(io::Result<IoSessionReadWrite>) + Send + 'static) {
        thread::spawn(move || {
            let result = self.start();
            ready(result);
        });
    }
}
