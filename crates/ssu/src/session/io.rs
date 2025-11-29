use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use tracing::error;

use crate::session::{
    IoSessionEndpoint, SessionEndpoint, SessionRecvEndpoint, SessionSendEndpoint, Ticked,
};

pub struct IoSession {
    pub tx: mpsc::Sender<u8>,
    pub rx: mpsc::Receiver<u8>,
}

pub struct IoSessionRecvEndpoint {
    pub rx: mpsc::Receiver<u8>,
}

pub struct IoSessionSendEndpoint {
    pub tx: mpsc::Sender<u8>,
}

impl SessionRecvEndpoint for IoSessionRecvEndpoint {
    fn recv(&mut self) -> Ticked {
        match self.rx.recv() {
            Ok(byte) => Ticked::Byte(byte),
            Err(e) => Ticked::Idle,
        }
    }
}

impl SessionSendEndpoint for IoSessionSendEndpoint {
    fn send(&mut self, b: u8) {
        match self.tx.send(b) {
            Ok(()) => {}
            Err(e) => {
                error!("Failed to send byte to TX: {}", e);
            }
        }
    }
}

impl IoSession {
    pub fn new(tx: mpsc::Sender<u8>, rx: mpsc::Receiver<u8>) -> Self {
        Self { tx, rx }
    }
}

impl SessionEndpoint for IoSession {
    fn recv(&mut self) -> Ticked {
        match self.rx.recv() {
            Ok(byte) => Ticked::Byte(byte),
            Err(e) => Ticked::Idle,
        }
    }
    fn send(&mut self, b: u8) {
        match self.tx.send(b) {
            Ok(()) => {}
            Err(e) => {
                error!("Failed to send byte to TX: {}", e);
            }
        }
    }

    fn split(
        self: Box<Self>,
    ) -> (
        Box<dyn SessionRecvEndpoint + Send + 'static>,
        Box<dyn SessionSendEndpoint + Send + 'static>,
    ) {
        (
            Box::new(IoSessionRecvEndpoint { rx: self.rx }),
            Box::new(IoSessionSendEndpoint { tx: self.tx }),
        )
    }
}

pub struct IoSessionReadWrite {
    pub reader: Box<dyn io::Read + Send + 'static>,
    pub writer: Box<dyn io::Write + Send + 'static>,
}

impl IoSessionReadWrite {
    pub fn new(
        reader: impl io::Read + Send + 'static,
        writer: impl io::Write + Send + 'static,
    ) -> Self {
        Self {
            reader: Box::new(reader),
            writer: Box::new(writer),
        }
    }
}

/// Boot an I/O session endpoint in various threads and return a
/// [`SessionEndpoint`] implementation.
pub fn boot_io(io: impl IoSessionEndpoint) -> Result<IoSession, std::io::Error> {
    let (tx, rx1) = mpsc::channel();
    let (tx2, rx) = mpsc::sync_channel(16);

    io.start(|session| {
        match session {
            Ok(session) => {
                let mut reader = session.reader;
                let mut writer = session.writer;
                let xoff = Arc::new(AtomicBool::new(false));

                let xoff_clone = xoff.clone();
                thread::spawn(move || {
                    let mut buf = [0; 1];
                    loop {
                        match reader.read_exact(&mut buf) {
                            Ok(_) => {}
                            Err(e) => {
                                error!("Failed to read byte from RX: {}", e);
                                break;
                            }
                        }
                        while xoff_clone.load(Ordering::Relaxed) {
                            // Spin wait for XOFF to be cleared
                            thread::sleep(Duration::from_millis(10));
                        }
                        match tx2.send(buf[0]) {
                            Ok(()) => {}
                            Err(e) => {
                                error!("Failed to send byte to RX: {}", e);
                                break;
                            }
                        }
                    }
                });

                thread::spawn(move || {
                    loop {
                        match rx1.recv() {
                            Ok(0x13) => {
                                xoff.store(true, Ordering::Relaxed);
                            }
                            Ok(0x11) => {
                                xoff.store(false, Ordering::Relaxed);
                            }
                            Ok(byte) => {
                                writer.write_all(&[byte]).unwrap();
                            }
                            Err(e) => {
                                error!("Failed to receive byte from TX: {}", e);
                                break;
                            }
                        }
                    }
                });
            }
            Err(e) => {
                error!("Failed to start IO session: {}", e);
            }
        }
    });
    Ok(IoSession { rx, tx })
}
