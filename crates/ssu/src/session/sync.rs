use std::io;
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

use crate::session::{SessionPartsUnsend, SessionRecvEndpoint, SessionSendEndpoint};

// SAFETY: This is a no-op waker with no functionality.
static NOOP_WAKER: Waker = unsafe { Waker::new(std::ptr::null(), NOOP_VTABLE) };
const NOOP_VTABLE: &RawWakerVTable =
    &RawWakerVTable::new(|x| RawWaker::new(x, NOOP_VTABLE), drop, drop, drop);

/// A session that can be used to send and receive bytes synchronously.
pub struct SyncSession {
    pub send: Box<dyn SessionSendEndpoint + 'static>,
    pub recv: Box<dyn SessionRecvEndpoint + 'static>,
}

impl SyncSession {
    pub fn new(parts: SessionPartsUnsend) -> Self {
        Self {
            send: parts.send,
            recv: parts.recv,
        }
    }

    pub fn try_send(&mut self, b: u8) -> Result<io::Result<()>, u8> {
        let ctx = &mut Context::from_waker(&NOOP_WAKER);
        match self.send.poll_send(ctx, b) {
            Poll::Ready(Ok(())) => Ok(Ok(())),
            Poll::Ready(Err(e)) => Ok(Err(e)),
            Poll::Pending => Err(b),
        }
    }

    pub fn try_recv(&mut self) -> io::Result<Option<u8>> {
        let ctx = &mut Context::from_waker(&NOOP_WAKER);
        match self.recv.poll_recv(ctx) {
            Poll::Ready(Ok(b)) => Ok(Some(b)),
            Poll::Ready(Err(e)) => Err(e),
            Poll::Pending => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{
        Session, io_session::boot_io, loopback::LoopbackConfig, pipe::AnonymousPipeConfig,
    };
    use std::{thread, time::Duration};

    /// Test that the loopback session works.
    #[test]
    fn test_sync_session_loopback() {
        let loopback = LoopbackConfig::default();
        let parts = loopback.boot().unwrap();
        let mut session = SyncSession::new(parts.into());
        assert_eq!(session.try_recv().unwrap(), None);
        session.try_send(b'A').unwrap().unwrap();
        assert_eq!(session.try_recv().unwrap(), Some(b'A'));
    }

    /// Test that an I/O-based loopback session works.
    #[test]
    fn test_sync_session_pipe() {
        let pipe = AnonymousPipeConfig::default();
        let parts = boot_io(pipe).unwrap();
        let mut session = SyncSession::new(parts.into());
        assert_eq!(session.try_recv().unwrap(), None);
        session.try_send(b'A').unwrap().unwrap();
        let mut found = false;
        for _ in 0..1000 {
            if let Some(_) = session.try_recv().unwrap() {
                found = true;
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(found);
    }
}
