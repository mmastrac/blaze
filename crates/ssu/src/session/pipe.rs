use std::fs::OpenOptions;
use std::io::{self, pipe};
use std::path::PathBuf;
use std::thread;

use crate::session::io_session::{IoSession, IoSessionReadWrite};

/// Create an anonymous pipe.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AnonymousPipeConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipeConfig {
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipesConfig {
    pub rx: PathBuf,
    pub tx: PathBuf,
}

impl AnonymousPipeConfig {
    fn start(self) -> io::Result<IoSessionReadWrite> {
        let (pipe_r, pipe_w) = pipe()?;
        Ok(IoSessionReadWrite::new(pipe_r, pipe_w))
    }
}

impl PipeConfig {
    fn start(self) -> io::Result<IoSessionReadWrite> {
        let pipe_r = OpenOptions::new().read(true).write(true).open(&self.path)?;
        let pipe_w = pipe_r.try_clone()?;
        Ok(IoSessionReadWrite::new(pipe_r, pipe_w))
    }
}

impl PipesConfig {
    fn start(self) -> io::Result<IoSessionReadWrite> {
        let pipe_r = OpenOptions::new().read(true).open(&self.rx)?;
        let pipe_w = OpenOptions::new().write(true).open(&self.tx)?;
        Ok(IoSessionReadWrite::new(pipe_r, pipe_w))
    }
}

impl IoSession for AnonymousPipeConfig {
    fn start(self, ready: impl FnOnce(io::Result<IoSessionReadWrite>) + Send + 'static) {
        thread::spawn(move || {
            let result = self.start();
            ready(result);
        });
    }
}

impl IoSession for PipeConfig {
    fn start(self, ready: impl FnOnce(io::Result<IoSessionReadWrite>) + Send + 'static) {
        thread::spawn(move || {
            let result = self.start();
            ready(result);
        });
    }
}

impl IoSession for PipesConfig {
    fn start(self, ready: impl FnOnce(io::Result<IoSessionReadWrite>) + Send + 'static) {
        thread::spawn(move || {
            let result = self.start();
            ready(result);
        });
    }
}
