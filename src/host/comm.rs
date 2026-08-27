use std::sync::mpsc::{Receiver, SyncSender, TrySendError};

use ssu::session::{SessionConfig, SessionPartsUnsend, SyncSession, xonoff::xonoff_unsend};

use crate::machine::generic::duart::DUARTChannel;

use tracing::{error, info};

/// Links a DUART channel's pipes to a synchronous session.
pub struct CommSession {
    session: SyncSession,
    rx: Receiver<u8>,
    tx: SyncSender<u8>,
    pending_rx: Option<u8>,
    pending_tx: Option<u8>,
    /// Set once that direction's endpoint is gone. Stops the polling and the
    /// logging. The two directions are separate channels with separate pump
    /// threads and die independently -- `exec` gives the child a piped stdin
    /// and a piped stdout, so it can close one and keep using the other --
    /// so a single flag would let a dead write side silence a live read side.
    send_closed: bool,
    recv_closed: bool,
}

impl CommSession {
    /// A disconnect is permanent: log it once and tell the caller to latch
    /// that direction. A real I/O error is logged and left open; the pump
    /// thread that reported it exits, so the next poll disconnects anyway.
    fn note_error(what: &str, e: &std::io::Error) -> bool {
        if e.kind() == std::io::ErrorKind::NotConnected {
            info!("Session {what} side disconnected, detaching from DUART channel");
            true
        } else {
            error!("Failed to {what} byte: {e}");
            false
        }
    }

    pub fn tick(&mut self) {
        // DUART's send to session's send
        let b = if self.send_closed {
            None
        } else if let Some(pending) = self.pending_rx.take() {
            Some(pending)
        } else {
            match self.rx.try_recv() {
                Ok(byte) => Some(byte),
                Err(_) => None,
            }
        };

        if let Some(byte) = b {
            match self.session.try_send(byte) {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    self.send_closed = Self::note_error("send", &e);
                }
                Err(b) => {
                    self.pending_rx.replace(b);
                }
            }
        }

        // Session's recv to DUART's recv
        let b = if let Some(pending) = self.pending_tx.take() {
            Some(pending)
        } else if self.recv_closed {
            None
        } else {
            match self.session.try_recv() {
                Ok(Some(byte)) => Some(byte),
                Ok(None) => None,
                Err(e) => {
                    self.recv_closed = Self::note_error("receive", &e);
                    None
                }
            }
        };

        if let Some(byte) = b {
            match self.tx.try_send(byte) {
                Ok(()) => {}
                Err(TrySendError::Full(byte)) => {
                    self.pending_tx.replace(byte);
                }
                Err(TrySendError::Disconnected(_)) => {}
            }
        }
    }
}

/// Connect a DUART channel to the configured communication method
pub fn connect_duart(
    channel: DUARTChannel,
    config: SessionConfig,
) -> Result<CommSession, std::io::Error> {
    let session = config.start_unsend()?;
    connect_session(channel, session)
}

/// Connect a DUART channel to a session
pub fn connect_session(
    channel: DUARTChannel,
    session: SessionPartsUnsend,
) -> Result<CommSession, std::io::Error> {
    let xonoff = xonoff_unsend(session);
    let session = SyncSession::new(xonoff);
    Ok(CommSession {
        session,
        rx: channel.rx,
        tx: channel.tx,
        pending_rx: None,
        pending_tx: None,
        send_closed: false,
        recv_closed: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssu::session::xonoff::XON;
    use ssu::session::{SessionPartsUnsend, SessionRecvEndpoint, SessionSendEndpoint};
    use std::cell::RefCell;
    use std::io;
    use std::rc::Rc;
    use std::task::{Context, Poll};

    /// A session whose two directions can be failed independently, so the
    /// tests can reproduce a child that closes stdin and keeps writing stdout.
    #[derive(Debug, Default)]
    struct Fake {
        send_polls: usize,
        recv_polls: usize,
        send_fails: bool,
        recv_fails: bool,
        sent: Vec<u8>,
        feed: usize,
    }

    fn not_connected() -> io::Error {
        io::Error::new(io::ErrorKind::NotConnected, "Channel disconnected")
    }

    #[derive(Debug)]
    struct FakeSend(Rc<RefCell<Fake>>);
    #[derive(Debug)]
    struct FakeRecv(Rc<RefCell<Fake>>);

    impl SessionSendEndpoint for FakeSend {
        fn poll_send(&mut self, _ctx: &mut Context<'_>, b: u8) -> Poll<io::Result<()>> {
            let mut f = self.0.borrow_mut();
            f.send_polls += 1;
            if f.send_fails {
                return Poll::Ready(Err(not_connected()));
            }
            f.sent.push(b);
            Poll::Ready(Ok(()))
        }
    }

    impl SessionRecvEndpoint for FakeRecv {
        fn poll_recv(&mut self, _ctx: &mut Context<'_>) -> Poll<io::Result<u8>> {
            let mut f = self.0.borrow_mut();
            f.recv_polls += 1;
            if f.recv_fails {
                return Poll::Ready(Err(not_connected()));
            }
            if f.feed == 0 {
                return Poll::Pending;
            }
            f.feed -= 1;
            Poll::Ready(Ok(b'x'))
        }
    }

    /// A comm session on a fake, with the xon/xoff gate already opened.
    fn rig() -> (CommSession, DUARTChannel, Rc<RefCell<Fake>>) {
        let fake = Rc::new(RefCell::new(Fake::default()));
        let (host, terminal) = DUARTChannel::new();
        let parts = SessionPartsUnsend::new(FakeSend(fake.clone()), FakeRecv(fake.clone()));
        let mut comm = connect_session(host, parts).unwrap();

        // The gate starts closed; it swallows the XON rather than forwarding it.
        terminal.tx.send(XON).unwrap();
        comm.tick();
        fake.borrow_mut().send_polls = 0;
        fake.borrow_mut().recv_polls = 0;
        (comm, terminal, fake)
    }

    /// The write side dies while the read side is still delivering. The
    /// terminal must keep receiving.
    #[test]
    fn dead_send_side_leaves_recv_alive() {
        let (mut comm, terminal, fake) = rig();
        fake.borrow_mut().send_fails = true;
        fake.borrow_mut().feed = 16;

        terminal.tx.send(b'A').unwrap(); // a keystroke, to trip the send path
        comm.tick();
        assert_eq!(fake.borrow().send_polls, 1);

        let mut received = 0;
        for _ in 0..32 {
            let _ = terminal.tx.try_send(b'B'); // keep typing at a dead write side
            comm.tick();
            while terminal.rx.try_recv().is_ok() {
                received += 1;
            }
        }
        assert_eq!(fake.borrow().send_polls, 1, "send polled after disconnect");
        assert_eq!(received, 16, "bytes did not reach the terminal");
    }

    /// The read side dies while the write side is still live. Keystrokes must
    /// keep getting out, and the dead side must be polled only once.
    #[test]
    fn dead_recv_side_leaves_send_alive() {
        let (mut comm, terminal, fake) = rig();
        fake.borrow_mut().recv_fails = true;

        comm.tick();
        assert_eq!(fake.borrow().recv_polls, 1);

        for _ in 0..8 {
            terminal.tx.send(b'C').unwrap();
            comm.tick();
        }
        assert_eq!(fake.borrow().recv_polls, 1, "recv polled after disconnect");
        assert_eq!(fake.borrow().sent.len(), 8, "keystrokes did not get out");
    }
}
