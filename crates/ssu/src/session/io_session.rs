use std::sync::Arc;
use std::sync::mpsc::{self, TryRecvError, TrySendError};
use std::task::{Context, Poll};
use std::{fmt, io, thread};

use atomic_waker::AtomicWaker;
use tracing::error;

use crate::session::{SessionParts, SessionRecvEndpoint, SessionSendEndpoint};

/// A I/O-based session endpoint that is started in a separate thread.
/// Backpressure is applied by SyncSender filling up at which point the endpoint
/// should apply hardware flow control signals if available.
pub trait IoSession {
    fn start(self, ready: impl FnOnce(io::Result<IoSessionReadWrite>) + Send + 'static);
}

pub struct IoSessionRecvEndpoint {
    rx: mpsc::Receiver<io::Result<u8>>,
    waker: Arc<AtomicWaker>,
}

impl fmt::Debug for IoSessionRecvEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "IoSessionRecvEndpoint")
    }
}

pub struct IoSessionSendEndpoint {
    tx: mpsc::SyncSender<u8>,
    waker: Arc<AtomicWaker>,
}

impl fmt::Debug for IoSessionSendEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "IoSessionSendEndpoint")
    }
}

impl SessionRecvEndpoint for IoSessionRecvEndpoint {
    fn poll_recv(&mut self, ctx: &mut Context<'_>) -> Poll<io::Result<u8>> {
        match self.rx.try_recv() {
            Ok(byte) => Poll::Ready(byte),
            Err(TryRecvError::Empty) => {
                self.waker.register(ctx.waker());
                Poll::Pending
            }
            Err(TryRecvError::Disconnected) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "Channel disconnected",
            ))),
        }
    }
}

impl SessionSendEndpoint for IoSessionSendEndpoint {
    fn poll_send(&mut self, ctx: &mut Context<'_>, b: u8) -> std::task::Poll<io::Result<()>> {
        match self.tx.try_send(b) {
            Ok(()) => Poll::Ready(Ok(())),
            Err(TrySendError::Full(_)) => {
                self.waker.register(ctx.waker());
                Poll::Pending
            }
            Err(TrySendError::Disconnected(_)) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "Channel disconnected",
            ))),
        }
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
pub fn boot_io(io: impl IoSession) -> Result<SessionParts, std::io::Error> {
    let (tx, rx1) = mpsc::sync_channel(16);
    let recv_waker = Arc::new(AtomicWaker::new());
    let (tx2, rx) = mpsc::sync_channel(16);
    let send_waker = Arc::new(AtomicWaker::new());

    let recv_waker_clone = recv_waker.clone();
    let send_waker_clone = send_waker.clone();
    io.start(|session| {
        start(rx1, recv_waker_clone, tx2, send_waker_clone, session);
    });

    Ok(SessionParts::new(
        IoSessionSendEndpoint {
            tx,
            waker: send_waker,
        },
        IoSessionRecvEndpoint {
            rx,
            waker: recv_waker,
        },
    ))
}

fn start(
    rx: mpsc::Receiver<u8>,
    rx_waker: Arc<AtomicWaker>,
    tx: mpsc::SyncSender<io::Result<u8>>,
    tx_waker: Arc<AtomicWaker>,
    session: Result<IoSessionReadWrite, io::Error>,
) {
    match session {
        Ok(session) => {
            let mut reader = session.reader;
            let mut writer = session.writer;

            thread::spawn(move || {
                let mut buf = [0; 1];
                loop {
                    match reader.read_exact(&mut buf) {
                        Ok(_) => {}
                        // TimedOut just means the line is idle.
                        Err(e)
                            if matches!(
                                e.kind(),
                                io::ErrorKind::TimedOut
                                    | io::ErrorKind::WouldBlock
                                    | io::ErrorKind::Interrupted
                            ) =>
                        {
                            continue;
                        }
                        Err(e) => {
                            match tx.send(Err(e)) {
                                Ok(()) => {}
                                Err(e) => {
                                    error!("Failed to send error to TX: {}", e);
                                    break;
                                }
                            }
                            break;
                        }
                    }
                    match tx.send(Ok(buf[0])) {
                        Ok(()) => {
                            rx_waker.wake();
                        }
                        Err(e) => {
                            error!("Failed to send byte to RX: {}", e);
                            break;
                        }
                    }
                }
            });

            thread::spawn(move || {
                loop {
                    match rx.recv() {
                        Ok(byte) => {
                            writer.write_all(&[byte]).unwrap();
                            tx_waker.wake();
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
}
