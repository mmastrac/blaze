use std::{io, sync::mpsc};

pub struct IoSession {
    pub reader: Box<dyn io::Read + Send + 'static>,
    pub writer: Box<dyn io::Write + Send + 'static>,

    pub rx: mpsc::SyncSender<u8>,
    pub tx: mpsc::Receiver<u8>,
}

impl IoSession {
    pub fn new(
        reader: impl io::Read + Send + 'static,
        writer: impl io::Write + Send + 'static,
        rx: mpsc::SyncSender<u8>,
        tx: mpsc::Receiver<u8>,
    ) -> Self {
        IoSession {
            reader: Box::new(reader),
            writer: Box::new(writer),
            rx,
            tx,
        }
    }
}
