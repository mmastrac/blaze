use std::{
    fmt, io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
};

use atomic_waker::AtomicWaker;
use tracing::debug;

use crate::session::{SessionParts, SessionPartsUnsend, SessionRecvEndpoint, SessionSendEndpoint};

pub const XON: u8 = 0x11;
pub const XOFF: u8 = 0x13;

pub fn xonoff(session: SessionParts) -> SessionParts {
    let xon = Arc::new(Xon::default());
    SessionParts::new(
        XonOffSend {
            xon: xon.clone(),
            inner: session.send,
        },
        XonOffRecv {
            xon,
            inner: session.recv,
        },
    )
}

pub fn xonoff_unsend(session: SessionPartsUnsend) -> SessionPartsUnsend {
    let xon = Arc::new(Xon::default());
    SessionPartsUnsend::new(
        XonOffSend {
            xon: xon.clone(),
            inner: session.send,
        },
        XonOffRecv {
            xon,
            inner: session.recv,
        },
    )
}

#[derive(Default)]
struct Xon {
    xon: AtomicBool,
    waker: AtomicWaker,
}

struct XonOffRecv<T: SessionRecvEndpoint> {
    xon: Arc<Xon>,
    inner: T,
}

struct XonOffSend<T: SessionSendEndpoint> {
    xon: Arc<Xon>,
    inner: T,
}

impl<T: SessionRecvEndpoint> fmt::Debug for XonOffRecv<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "XonOffRecv(xon={:?}, inner={:?})",
            self.xon.xon.load(Ordering::Acquire),
            self.inner
        )
    }
}

impl<T: SessionRecvEndpoint> SessionRecvEndpoint for XonOffRecv<T> {
    fn poll_recv(&mut self, ctx: &mut Context<'_>) -> Poll<io::Result<u8>> {
        if self.xon.xon.load(Ordering::Acquire) {
            self.inner.poll_recv(ctx)
        } else {
            self.xon.waker.register(ctx.waker());
            Poll::Pending
        }
    }
}

impl<T: SessionSendEndpoint> fmt::Debug for XonOffSend<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "XonOffSend(xon={:?}, inner={:?})",
            self.xon.xon.load(Ordering::Acquire),
            self.inner
        )
    }
}

impl<T: SessionSendEndpoint> SessionSendEndpoint for XonOffSend<T> {
    fn poll_send(&mut self, ctx: &mut Context<'_>, b: u8) -> Poll<io::Result<()>> {
        match b {
            XON => {
                debug!("XON");
                self.xon.xon.store(true, Ordering::Release);
                self.xon.waker.wake();
                Poll::Ready(Ok(()))
            }
            XOFF => {
                debug!("XOFF");
                self.xon.xon.store(false, Ordering::Release);
                Poll::Ready(Ok(()))
            }
            _ => self.inner.poll_send(ctx, b),
        }
    }
}
