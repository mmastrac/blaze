use std::{fs::OpenOptions, io, path::PathBuf, sync::mpsc, thread};

use crate::session::{IoSessionEndpoint, io::IoSession};

pub struct SinglePipeSession {
    path: PathBuf,
}

impl SinglePipeSession {
    pub fn new(path: PathBuf) -> Self {
        SinglePipeSession { path }
    }

    fn start(self, rx: mpsc::SyncSender<u8>, tx: mpsc::Receiver<u8>) -> io::Result<IoSession> {
        let pipe_r = OpenOptions::new().read(true).write(true).open(&self.path)?;
        let pipe_w = pipe_r.try_clone()?;
        Ok(IoSession::new(pipe_r, pipe_w, rx, tx))
    }
}

impl IoSessionEndpoint for SinglePipeSession {
    fn start(
        self,
        rx: mpsc::SyncSender<u8>,
        tx: mpsc::Receiver<u8>,
        ready: impl FnOnce(io::Result<IoSession>) + Send + 'static,
    ) {
        thread::spawn(move || {
            let result = self.start(rx, tx);
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

    fn start(self, rx: mpsc::SyncSender<u8>, tx: mpsc::Receiver<u8>) -> io::Result<IoSession> {
        let pipe_r = OpenOptions::new().read(true).open(&self.recv)?;
        let pipe_w = OpenOptions::new().write(true).open(&self.send)?;
        Ok(IoSession::new(pipe_r, pipe_w, rx, tx))
    }
}

impl IoSessionEndpoint for DualPipeSession {
    fn start(
        self,
        rx: mpsc::SyncSender<u8>,
        tx: mpsc::Receiver<u8>,
        ready: impl FnOnce(io::Result<IoSession>) + Send + 'static,
    ) {
        thread::spawn(move || {
            let result = self.start(rx, tx);
            ready(result);
        });
    }
}
