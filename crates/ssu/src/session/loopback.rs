use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::{fmt, io};

use atomic_waker::AtomicWaker;

use crate::session::{Session, SessionParts, SessionRecvEndpoint, SessionSendEndpoint};

/// Creates a pair of wakeable queues.
pub fn wakeable_queue() -> (WakeableQueueSend, WakeableQueueRecv) {
    let queue = Arc::new(Mutex::new(VecDeque::new()));
    let waker = Arc::new(AtomicWaker::new());
    (
        WakeableQueueSend {
            queue: queue.clone(),
            waker: waker.clone(),
        },
        WakeableQueueRecv {
            queue: queue.clone(),
            waker: waker.clone(),
        },
    )
}

#[derive(Clone)]
pub struct WakeableQueueSend {
    queue: Arc<Mutex<VecDeque<u8>>>,
    waker: Arc<AtomicWaker>,
}

pub struct WakeableQueueRecv {
    queue: Arc<Mutex<VecDeque<u8>>>,
    waker: Arc<AtomicWaker>,
}

impl fmt::Debug for WakeableQueueRecv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "WakeableQueueRecv(len={})",
            self.queue.lock().unwrap().len()
        )
    }
}

impl fmt::Debug for WakeableQueueSend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "WakeableQueueSend(len={})",
            self.queue.lock().unwrap().len()
        )
    }
}

impl SessionRecvEndpoint for WakeableQueueRecv {
    fn poll_recv(&mut self, ctx: &mut Context<'_>) -> Poll<io::Result<u8>> {
        match self.queue.lock().unwrap().pop_front() {
            Some(b) => Poll::Ready(Ok(b)),
            None => {
                self.waker.register(ctx.waker());
                Poll::Pending
            }
        }
    }
}

impl WakeableQueueSend {
    pub fn send(&self, b: u8) {
        self.queue.lock().unwrap().push_back(b);
        self.waker.wake();
    }

    pub fn send_bytes(&self, buf: &[u8]) {
        let mut queue = self.queue.lock().unwrap();
        queue.extend(buf);
        self.waker.wake();
    }
}

impl SessionSendEndpoint for WakeableQueueSend {
    fn poll_send(&mut self, _ctx: &mut Context<'_>, b: u8) -> Poll<io::Result<()>> {
        self.queue.lock().unwrap().push_back(b);
        self.waker.wake();
        Poll::Ready(Ok(()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LoopbackConfig {
    pub initial: String,
}

pub struct LoopbackRecvSession {
    queue: WakeableQueueRecv,
}

pub struct LoopbackSendSession {
    queue: WakeableQueueSend,
}

impl Session for LoopbackConfig {
    fn boot(self) -> std::io::Result<SessionParts> {
        let (send, recv) = wakeable_queue();
        Ok(SessionParts::new(
            LoopbackSendSession { queue: send },
            LoopbackRecvSession { queue: recv },
        ))
    }
}

impl SessionRecvEndpoint for LoopbackRecvSession {
    fn poll_recv(&mut self, ctx: &mut Context<'_>) -> Poll<io::Result<u8>> {
        self.queue.poll_recv(ctx)
    }
}

impl fmt::Debug for LoopbackRecvSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LoopbackRecvSession")
    }
}

impl SessionSendEndpoint for LoopbackSendSession {
    fn poll_send(&mut self, ctx: &mut Context<'_>, b: u8) -> Poll<io::Result<()>> {
        self.queue.poll_send(ctx, b)
    }
}

impl fmt::Debug for LoopbackSendSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LoopbackSendSession")
    }
}
