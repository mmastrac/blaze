use std::array;
use std::future::{Future, poll_fn};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll, Waker};

use tracing::{error, info, trace, warn};

use crate::buffer::{RingBufferHandle, SyncRingBuffer};
use crate::ops::{
    INTRO, MAX_COMMAND_LEN, MAX_LABEL_LEN, SSUOp, SSUOpcode, SSUState, SSUString, TERM,
};

/// The maximum number of bytes to send in a single chunk before
/// trying to poll the other session.
const FAIRNESS_CHUNK_SIZE: usize = 32;

/// The minimum number of credits to keep to avoid starvation. When we detect
/// the peer has reached this level, we top up the credit balance.
const RECV_CREDITS_LOW_WATER_MARK: usize = 128;
/// The amount of credits to top up the credit balance when the peer has reached
/// the low water mark. Note that this is smaller than the actual buffer.
const RECV_CREDITS_TOP_UP: usize = PEER_TO_SESSION_SIZE - 1024;

/// The maximum internal buffer size to receive from a session regardless of
/// upstream credit count. Note that the underlying session channel may have
/// additional buffering.
///
/// This works out to be a little less than half a second of data at 30k baud.
const SESSION_TO_PEER_SIZE: usize = 1024;
/// The maximum internal buffer size to queue for a session. This affects the
/// credit messages sent to the upstream peer and is effectively what we promise
/// to buffer per session to the upstream peer.
const PEER_TO_SESSION_SIZE: usize = 16 * 1024;

/// The maximum internal buffer size to queue for the peer's command queue.
const PEER_COMMAND_QUEUE_SIZE: usize = 32;

/// The maximum number of sessions to support. This matches the number of
/// sessions supported by the DEC VT525.
const MAX_SESSION_COUNT: usize = 4;

/// SSU server state machine implementation. This speaks SSU to a peer and
/// provides multiple SSU endpoints as sessions.
///
/// Data is both pushed and pulled externally for the peer and individual
/// sessions, while this server manages the credits and routing for data.
pub struct Server {
    /// The command queue for the peer.
    xon: Xon,
    max_sessions: u8,
    enabled: bool,
}

impl Server {
    pub fn new(max_sessions: u8) -> ServerHandles {
        // This is shared by a number of actors in the system
        let outgoing_command_queue = RingBufferHandle::default();
        outgoing_command_queue.push_sync(SSUOp::Probe(SSUState::Disabled, 1, max_sessions));

        let server = Server {
            xon: Xon::new(),
            max_sessions,
            enabled: false,
        };

        let server = ServerHandle {
            server: Arc::new(Mutex::new(server)),
        };

        let credits = array::from_fn(|_| Arc::new(Credits::new()));

        let server_read = {
            let server = server.clone();
            ServerRead {
                active_session_to_peer: None,
                outgoing_command_queue: outgoing_command_queue.clone(),
                outgoing_command_queue_bytes: Default::default(),
                server,
                fairness_counter: 0,
                sessions: array::from_fn(|_| RingBufferHandle::default()),
                credits: credits.clone(),
            }
        };

        let server_write = ServerWrite {
            active_session_from_peer: None,
            server: server.clone(),
            incoming_command_queue: Default::default(),
            outgoing_command_queue,
            sessions: array::from_fn(|_| RingBufferHandle::default()),
            credits,
        };

        let sessions = array::from_fn(|i| {
            (
                SessionRead {
                    buffer: server_write.sessions[i].clone(),
                },
                SessionWrite {
                    buffer: server_read.sessions[i].clone(),
                },
            )
        });

        ServerHandles {
            server_read,
            server_write,
            session_handles: SessionHandles {
                sessions,
                count: max_sessions as _,
            },
        }
    }
}

pub struct ServerHandles {
    pub server_read: ServerRead,
    pub server_write: ServerWrite,
    pub session_handles: SessionHandles,
}

/// The handles for the sessions.
pub struct SessionHandles {
    sessions: [(SessionRead, SessionWrite); MAX_SESSION_COUNT],
    count: usize,
}

impl IntoIterator for SessionHandles {
    type Item = (SessionRead, SessionWrite);
    type IntoIter = std::iter::Take<std::array::IntoIter<Self::Item, MAX_SESSION_COUNT>>;
    fn into_iter(self) -> Self::IntoIter {
        self.sessions.into_iter().take(self.count)
    }
}

enum RecvResult {
    NoData,
    NoCredits,
    Data(u8),
}

#[derive(Clone)]
struct ServerHandle {
    server: Arc<Mutex<Server>>,
}

impl ServerHandle {
    pub fn set_xon(&self, xon: bool) {
        self.server.lock().unwrap().xon.set_xon(xon);
    }

    pub async fn wait_xon(&self) {
        loop {
            let waker = {
                let lock = self.lock();
                if lock.xon.xon {
                    return;
                }
                lock.xon.waker.clone()
            };
            waker.wait().await;
        }
    }

    fn lock(&self) -> MutexGuard<Server> {
        self.server.lock().unwrap()
    }

    fn max_session(&self) -> u8 {
        self.lock().max_sessions
    }
}

#[cfg(feature = "server")]
pub async fn run_async(sessions: Vec<Box<dyn SessionEndpoint + Send + 'static>>) {
    use std::os::fd::AsFd;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use crate::session::Ticked;

    trace!("Entering run_async");
    let server = Server::new(sessions.len() as u8);
    let mut server_read = server.server_read;
    let owned = std::io::stdout().as_fd().try_clone_to_owned().unwrap();
    let file = std::fs::File::from(owned);
    let mut stdout = tokio::fs::File::from_std(file);
    tokio::task::spawn(async move {
        trace!("Entering stdout");
        loop {
            let Ok(b) = server_read.read().await else {
                return;
            };
            trace!("Writing byte {b:02X} to stdout");
            let Ok(_) = stdout.write_u8(b).await else {
                return;
            };
        }
    });

    let mut server_write = server.server_write;
    let mut stdin = tokio::io::stdin();
    tokio::task::spawn(async move {
        loop {
            let Ok(b) = stdin.read_u8().await else {
                return;
            };
            trace!("Got byte {b:02X} from stdin");
            let Ok(_) = server_write.write(b).await else {
                return;
            };
        }
    });

    for ((mut read, mut write), session) in server.session_handles.into_iter().zip(sessions) {
        let (mut sread, mut swrite) = session.split();
        let (tx, mut rx) = tokio::sync::mpsc::channel::<u8>(1);
        tokio::task::spawn_blocking(move || {
            loop {
                let b = rx.blocking_recv().unwrap();
                swrite.send(b);
            }
        });
        tokio::task::spawn(async move {
            trace!("Session task started");
            loop {
                let Ok(b) = read.read().await else {
                    break;
                };
                let Ok(_) = tx.send(b).await else {
                    break;
                };
            }
            trace!("Session task exited");
        });

        let (tx, mut rx) = tokio::sync::mpsc::channel::<u8>(1);
        tokio::task::spawn_blocking(move || {
            loop {
                match sread.recv() {
                    Ticked::Byte(b) => {
                        let Ok(_) = tx.blocking_send(b) else {
                            break;
                        };
                    }
                    Ticked::IdleInput | Ticked::Idle => {
                        std::thread::sleep(Duration::from_millis(10));
                        break;
                    }
                }
            }
        });
        tokio::task::spawn(async move {
            loop {
                let Some(b) = rx.recv().await else {
                    break;
                };
                trace!("Got byte {b:02X} from session, writing to peer");
                let Ok(_) = write.write(b).await else {
                    break;
                };
            }
        });
    }

    loop {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[derive(Default)]
pub(crate) struct WakerHandle {
    registering: AtomicBool,
    waker: Mutex<Option<Waker>>,
}

impl WakerHandle {
    pub fn new() -> Self {
        Self {
            waker: Default::default(),
            registering: AtomicBool::new(false),
        }
    }

    pub fn wait(&self) -> impl Future<Output = ()> + '_ {
        assert!(self.waker.lock().unwrap().is_none());
        assert!(!self.registering.swap(true, Ordering::AcqRel));
        poll_fn(|cx| {
            if self.registering.swap(false, Ordering::AcqRel) {
                assert!(self.waker.lock().unwrap().is_none());
                self.waker.lock().unwrap().replace(cx.waker().clone());
                Poll::Pending
            } else if self.waker.lock().unwrap().is_some() {
                Poll::Pending
            } else {
                Poll::Ready(())
            }
        })
    }

    pub fn register(&self, cx: &mut Context<'_>) {
        self.waker.lock().unwrap().replace(cx.waker().clone());
    }

    pub fn maybe_wake(&self) {
        if let Some(waker) = self.waker.lock().unwrap().take() {
            waker.wake();
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CreditsSlot {
    SessionToPeer,
    PeerToSession,
}

#[derive(Default)]
struct Credits {
    session_to_peer: AtomicI32,
    peer_to_session: AtomicI32,
    waker: WakerHandle,
}

impl Credits {
    pub fn new() -> Self {
        Self {
            session_to_peer: AtomicI32::new(0),
            peer_to_session: AtomicI32::new(0),
            waker: WakerHandle::new(),
        }
    }

    fn slot(&self, slot: CreditsSlot) -> &AtomicI32 {
        match slot {
            CreditsSlot::SessionToPeer => &self.session_to_peer,
            CreditsSlot::PeerToSession => &self.peer_to_session,
        }
    }

    pub fn add(&self, slot: CreditsSlot, count: usize) {
        self.slot(slot).fetch_add(count as _, Ordering::AcqRel);
        if slot == CreditsSlot::SessionToPeer {
            self.waker.maybe_wake();
        }
    }

    pub fn zero_all(&self) {
        self.session_to_peer.store(0, Ordering::Release);
        self.peer_to_session.store(0, Ordering::Release);
        self.waker.maybe_wake();
    }

    pub fn zero(&self, slot: CreditsSlot) {
        self.slot(slot).store(0, Ordering::Release);
    }

    pub fn get(&self, slot: CreditsSlot) -> usize {
        self.slot(slot).load(Ordering::Acquire).max(0) as usize
    }

    pub fn available(&self, slot: CreditsSlot) -> bool {
        self.get(slot) > 0
    }

    pub fn try_take_one(&self, slot: CreditsSlot) -> Result<usize, ()> {
        let res = self.slot(slot).fetch_sub(1, Ordering::AcqRel);
        if res <= 0 {
            self.slot(slot).fetch_add(1, Ordering::AcqRel);
            Err(())
        } else {
            Ok(res as usize)
        }
    }
}

struct Xon {
    waker: Arc<WakerHandle>,
    xon: bool,
}

impl Xon {
    pub fn new() -> Self {
        Self {
            waker: Arc::new(WakerHandle::new()),
            xon: true,
        }
    }

    pub fn set_xon(&mut self, xon: bool) {
        self.xon = xon;
        if xon {
            self.waker.maybe_wake();
        }
    }
}

pub struct SessionRead {
    buffer: RingBufferHandle<PEER_TO_SESSION_SIZE, u8>,
}

impl SessionRead {
    /// Reads a byte queued from the peer to send to the session.
    pub async fn read(&mut self) -> Result<u8, SessionError> {
        Ok(self.buffer.pop().await)
    }
}

pub struct SessionWrite {
    buffer: RingBufferHandle<SESSION_TO_PEER_SIZE, u8>,
}

impl SessionWrite {
    /// Writes a byte received from the session.
    ///
    /// If this function returns `Pending`, the caller should disable hardware
    /// flow control line to apply backpressure upstream.
    pub async fn write(&mut self, b: u8) -> Result<(), SessionError> {
        self.buffer.push(b).await;
        Ok(())
    }
}

pub enum SessionError {
    Closed,
}

pub struct ServerRead {
    /// Track the active session we're sending data to on the peer side.
    ///
    /// NOTE: The peer and server may have different active sessions.
    active_session_to_peer: Option<u8>,
    sessions: [RingBufferHandle<SESSION_TO_PEER_SIZE, u8>; MAX_SESSION_COUNT],
    credits: [Arc<Credits>; MAX_SESSION_COUNT],

    outgoing_command_queue: RingBufferHandle<PEER_COMMAND_QUEUE_SIZE, SSUOp<0>>,
    outgoing_command_queue_bytes: SyncRingBuffer<MAX_COMMAND_LEN, u8>,
    fairness_counter: usize,
    server: ServerHandle,
}

impl ServerRead {
    /// Reads a byte to send to the peer. The peer must be in XON state.
    ///
    /// If there are commands in the command queue, we always send those first.
    ///
    /// Otherwise, we read from the select command buffer.
    ///
    /// If neither of those have queued bytes, we read from the session buffers,
    /// potentially activating a different session first depending on credits
    /// and fairness.
    pub async fn read(&mut self) -> Result<u8, ServerError> {
        trace!("Entering read");

        // If XON arrives in the peer stream unencoded, we treat that as an
        // emergency stop for all comms.
        self.server.wait_xon().await;
        trace!("XON");

        'read: loop {
            // Outgoing commands jump the queue
            'command: loop {
                if let Some(b) = self.outgoing_command_queue_bytes.pop() {
                    trace!("Sending command byte: {b:02X}");
                    return Ok(b);
                }
                loop {
                    if let Some(op) = self.outgoing_command_queue.pop_sync() {
                        let mut buf = [0; MAX_COMMAND_LEN];
                        match op.serialize(&mut buf) {
                            Ok(buf) => {
                                // Successfully serialized the command, so we can
                                // send it to the peer.
                                debug_assert!(self.outgoing_command_queue_bytes.is_empty());
                                trace!(
                                    "Sending command: {op:?} as {:?}",
                                    String::from_utf8_lossy(buf)
                                );
                                self.outgoing_command_queue_bytes.replace_with_slice(buf);
                                continue 'command;
                            }
                            Err(e) => {
                                // This should never happen - log and try again.
                                // TODO: It might be better if we just closed this
                                // session instead.
                                error!("Unexpected error serializing internal command: {e:?}");
                            }
                        }
                    } else {
                        break 'command;
                    }
                }
            }

            if self.server.lock().enabled {
                // There are no outgoing commands, so check for data. This is
                // curently not a fully-optimal or fair algorithm.
                trace!("Checking sessions");

                // First, check the active session. We'll read up to
                // FAIRNESS_CHUNK_SIZE bytes from the active session before giving
                // another session a chance.
                if self.fairness_counter > 0 {
                    if let Some(active) = self.active_session_to_peer {
                        self.fairness_counter = self.fairness_counter.saturating_sub(1);
                        match self.try_recv(active) {
                            RecvResult::Data(b) => return Ok(b),
                            RecvResult::NoData => {
                                self.fairness_counter = 0;
                            }
                            RecvResult::NoCredits => {
                                // We should not hit this case because the
                                // remote end should have granted us more, but
                                // it's possible we've lost data on the line
                                // somewhere. We'll try to fix this up later.
                                self.fairness_counter = 0;
                            }
                        }
                    }
                }

                // If the action session is idle or out of credits, we'll check
                // the other sessions. To be fair here we should keep some sort
                // of LRU list.
                for i in 0..self.server.max_session() {
                    match self.try_recv(i) {
                        RecvResult::Data(b) => {
                            self.fairness_counter = FAIRNESS_CHUNK_SIZE;
                            if self.active_session_to_peer != Some(i) {
                                trace!("Activating session {i}");
                                self.active_session_to_peer = Some(i);
                                let mut buf = [0; MAX_COMMAND_LEN];
                                debug_assert!(self.outgoing_command_queue.is_empty());
                                debug_assert!(self.outgoing_command_queue_bytes.is_empty());
                                self.outgoing_command_queue_bytes.replace_with_slice(
                                    SSUOp::<0>::Select(i).serialize(&mut buf).unwrap(),
                                );
                                // We already have this byte, so send it as part of
                                // the command queue. This is valid because the
                                // command queue is just straight bytes.
                                self.outgoing_command_queue_bytes.push(b);
                                continue 'read;
                            }
                            trace!("Sending session {i} byte: {b:02X}");
                            return Ok(b);
                        }
                        RecvResult::NoData => {}
                        RecvResult::NoCredits => {
                            // We should not hit this case because the remote
                            // end should have granted us more, but it's
                            // possible we've lost data on the line somewhere.
                            let mut buf = [0; MAX_COMMAND_LEN];
                            // self.outgoing_command_queue_bytes
                            //     .replace_with_slice(SSUOp::<0>::Verify(i).serialize(&mut buf).unwrap());
                            continue; // 'read;
                        }
                    }
                }

                trace!("IDLE: Waiting for commands, or any session to yield data");
            }

            poll_fn(|cx| {
                // Check the command queue first
                if !self.outgoing_command_queue.is_empty() {
                    return Poll::Ready(());
                }
                self.outgoing_command_queue.register(cx);

                // Poll each session in turn
                for (session, credits) in self.sessions.iter().zip(&self.credits) {
                    if !session.is_empty() {
                        if credits.available(CreditsSlot::SessionToPeer) {
                            return Poll::Ready(());
                        } else {
                            trace!("No credits available for session, registering waker");
                            credits.waker.register(cx);
                        }
                    } else {
                        trace!("Session is empty, registering waker");
                        session.register(cx);
                    }
                }

                Poll::Pending
            })
            .await;
        }
    }

    fn try_recv(&mut self, session_id: u8) -> RecvResult {
        let session = &mut self.sessions[session_id as usize];
        if session.is_empty() {
            return RecvResult::NoData;
        }
        let Ok(credits) =
            self.credits[session_id as usize].try_take_one(CreditsSlot::SessionToPeer)
        else {
            trace!("No credits remaining for session {session_id}");
            return RecvResult::NoCredits;
        };
        trace!("Credits remaining: {credits}");
        match session.pop_sync() {
            Some(b) => RecvResult::Data(b),
            None => RecvResult::NoData,
        }
    }
}

pub struct ServerWrite {
    /// Track the active session we're receiving data from on the peer side.
    ///
    /// NOTE: The peer and server may have different active sessions.
    active_session_from_peer: Option<u8>,
    sessions: [RingBufferHandle<PEER_TO_SESSION_SIZE, u8>; MAX_SESSION_COUNT],
    credits: [Arc<Credits>; MAX_SESSION_COUNT],

    incoming_command_queue: SyncRingBuffer<MAX_COMMAND_LEN, u8>,
    outgoing_command_queue: RingBufferHandle<PEER_COMMAND_QUEUE_SIZE, SSUOp<0>>,
    server: ServerHandle,
}

impl ServerWrite {
    /// Writes a byte from the peer. If the internal buffers for the sessions
    /// are full, will be in the pending state.
    pub async fn write(&mut self, mut b: u8) -> Result<(), ServerError> {
        trace!("Writing byte {b:02X}");
        match b {
            0x11 => {
                trace!("Setting XON");
                self.server.set_xon(true);
            }
            0x13 => {
                trace!("Setting XOFF");
                self.server.set_xon(false);
            }
            INTRO => {
                // Always reset the command queue on INTRO
                trace!("Got INTRO");
                self.incoming_command_queue.clear();
                self.incoming_command_queue.push(INTRO);
            }
            TERM => {
                trace!("Got TERM");
                if !self.incoming_command_queue.is_empty() {
                    self.incoming_command_queue.push(TERM);
                    // If the command queue is full, we overflowed
                    if !self.incoming_command_queue.is_full() {
                        let mut op = [0; MAX_COMMAND_LEN];
                        let op_buf = self.incoming_command_queue.copy_into_slice(&mut op);
                        let op = SSUOp::<MAX_LABEL_LEN>::parse(&op_buf);
                        trace!(
                            "Parsed command: {op:?} from {:?}",
                            String::from_utf8_lossy(op_buf)
                        );
                        if let Ok(op) = op {
                            self.process_op(op).await;
                        }
                    }
                    self.incoming_command_queue.clear();
                }
            }
            _ => {
                // Control chars may be encoded as Ctrl+T + letter
                if self.incoming_command_queue.len() == 1 && (b'A'..=b'Z').contains(&b) {
                    self.incoming_command_queue.clear();
                    b = b.saturating_sub(b'@');
                }

                if !self.incoming_command_queue.is_empty() {
                    self.incoming_command_queue.push(b);
                    trace!(
                        "Pushing byte {b:02X} to command queue (len = {})",
                        self.incoming_command_queue.len()
                    );
                    if self.incoming_command_queue.is_full() {
                        error!("Overlong command received, discarding");
                        self.incoming_command_queue.clear();
                    }
                } else {
                    if let Some(session_id) = self.active_session_from_peer {
                        trace!("Pushing byte {b:02X} to active session");
                        self.sessions[session_id as usize].push(b).await;
                        if let Some(credits) = self.session_needs_credits(session_id) {
                            trace!("Adding {credits} credits to session {session_id}");
                            self.outgoing_command_queue
                                .push(SSUOp::AddCredits {
                                    session_id,
                                    credits,
                                })
                                .await;
                        }
                    } else {
                        // These bytes are unallocated and go to the bit bucket
                        trace!("Discarding byte {b:02X}");
                    }
                }
            }
        }
        Ok(())
    }

    async fn process_op(&mut self, op: SSUOp<MAX_LABEL_LEN>) {
        match op {
            SSUOp::Probe(SSUState::Disabled, protocol_variant, max_sessions) => {
                self.server.lock().enabled = false;
                self.incoming_command_queue.clear();
                self.outgoing_command_queue.clear();
                self.active_session_from_peer = None;
                for session in &self.sessions {
                    session.clear();
                }
                for credit in &self.credits {
                    credit.zero_all();
                }

                let max_sessions = max_sessions.max(self.server.max_session());
                self.outgoing_command_queue
                    .push(SSUOp::Probe(
                        SSUState::EnabledWithSessions,
                        protocol_variant,
                        max_sessions,
                    ))
                    .await;
            }
            SSUOp::Probe(
                SSUState::Enabled | SSUState::EnabledWithSessions,
                protocol_variant,
                max_sessions,
            ) => {
                self.server.lock().enabled = false;
                self.incoming_command_queue.clear();
                self.outgoing_command_queue.clear();
                self.active_session_from_peer = None;
                for session in &self.sessions {
                    session.clear();
                }
                for credit in &self.credits {
                    credit.zero_all();
                }

                let max_sessions = max_sessions.max(self.server.max_session());
                self.outgoing_command_queue
                    .push(SSUOp::Report {
                        op: SSUOpcode::Probe,
                        session_id: None,
                        code: 0,
                    })
                    .await;
                self.outgoing_command_queue
                    .push(SSUOp::Probe(
                        SSUState::EnabledWithSessions,
                        protocol_variant,
                        max_sessions,
                    ))
                    .await;
            }
            SSUOp::Disable => {
                self.outgoing_command_queue
                    .push(SSUOp::Report {
                        op: SSUOpcode::Disable,
                        session_id: None,
                        code: 0,
                    })
                    .await;
                self.server.lock().enabled = false;
            }
            SSUOp::Select(session_id) => {
                self.active_session_from_peer = Some(session_id);
                self.outgoing_command_queue
                    .push(SSUOp::Report {
                        op: SSUOpcode::Select,
                        session_id: Some(session_id),
                        code: 0,
                    })
                    .await;
                // Pre-emptively add credits to the session if needed
                if let Some(credits) = self.session_needs_credits(session_id) {
                    trace!("Adding {credits} credits to session {session_id}");
                    self.outgoing_command_queue
                        .push(SSUOp::AddCredits {
                            session_id,
                            credits,
                        })
                        .await;
                }
            }
            SSUOp::AddCredits {
                session_id,
                credits,
            } => {
                self.credits[session_id as usize].add(CreditsSlot::SessionToPeer, credits);
            }
            SSUOp::Zero(session_id) => {
                self.credits[session_id as usize].zero(CreditsSlot::SessionToPeer);
                self.outgoing_command_queue
                    .push(SSUOp::Report {
                        op: SSUOpcode::Zero,
                        session_id: Some(session_id),
                        code: 0,
                    })
                    .await;
            }
            SSUOp::Open { session_id, label } => {
                info!("Opening session {label:?}",);
                self.outgoing_command_queue
                    .push(SSUOp::Report {
                        op: SSUOpcode::Open,
                        session_id: Some(session_id),
                        code: 0,
                    })
                    .await;
            }
            SSUOp::Reset(session_id) => {
                self.outgoing_command_queue
                    .push(SSUOp::Report {
                        op: SSUOpcode::Reset,
                        session_id,
                        code: 0,
                    })
                    .await;
                if let Some(session_id) = session_id {
                    self.credits[session_id as usize].zero_all();
                } else {
                    for i in 0..self.server.max_session() {
                        self.credits[i as usize].zero_all();
                    }
                }
            }
            SSUOp::Verify(session_id) => {
                self.outgoing_command_queue
                    .push(SSUOp::Report {
                        op: SSUOpcode::Verify,
                        session_id: Some(session_id),
                        code: 0,
                    })
                    .await;

                // Restore the current peer credits to the session
                self.credits[session_id as usize].zero(CreditsSlot::PeerToSession);
                self.outgoing_command_queue
                    .push(SSUOp::Zero(session_id))
                    .await;

                // Top up the peer's credits to the session
                let remaining = self.sessions[session_id as usize].free();
                let credits = remaining.min(RECV_CREDITS_TOP_UP);
                self.credits[session_id as usize].add(CreditsSlot::PeerToSession, credits);

                self.outgoing_command_queue
                    .push(SSUOp::AddCredits {
                        session_id,
                        credits,
                    })
                    .await;
            }
            SSUOp::RequestRestore => {
                self.outgoing_command_queue
                    .push(SSUOp::Report {
                        op: SSUOpcode::RequestRestore,
                        session_id: None,
                        code: 0,
                    })
                    .await;
                self.outgoing_command_queue.push(SSUOp::Restore).await;
            }
            SSUOp::Report {
                op: SSUOpcode::Probe,
                session_id: None,
                code: _,
            } => {
                // TODO: We can use this to detect a dead peer
            }
            SSUOp::Report {
                op: SSUOpcode::Restore,
                session_id: None,
                code: _,
            } => {
                self.outgoing_command_queue
                    .push(SSUOp::Open {
                        session_id: 0,
                        label: SSUString::External(&[]),
                    })
                    .await;
            }
            SSUOp::Report {
                op: SSUOpcode::Open,
                session_id: Some(session_id),
                code: _,
            } => {
                // Set the credits to the low water mark so there's something.
                // When the peer starts sending data, we'll top them up.
                let credits = RECV_CREDITS_LOW_WATER_MARK;
                self.credits[session_id as usize].zero(CreditsSlot::PeerToSession);
                self.credits[session_id as usize].add(CreditsSlot::PeerToSession, credits);
                self.outgoing_command_queue
                    .push(SSUOp::AddCredits {
                        session_id,
                        credits,
                    })
                    .await;
                if session_id == self.server.max_session() - 1 {
                    self.outgoing_command_queue.push(SSUOp::RestoreEnd).await;
                } else {
                    self.outgoing_command_queue
                        .push(SSUOp::Open {
                            session_id: session_id + 1,
                            label: SSUString::default(),
                        })
                        .await;
                }
            }
            SSUOp::Report {
                op: SSUOpcode::Close,
                session_id: Some(_session_id),
                code: _,
            } => {
                // TODO: We can use this to detect a dead peer
            }
            SSUOp::Report {
                op: SSUOpcode::RestoreEnd,
                session_id: None,
                code: _,
            } => {
                // TODO: We can use this to detect a dead peer
                self.server.lock().enabled = true;
            }
            SSUOp::Report {
                op: SSUOpcode::Verify,
                session_id: Some(_session_id),
                code: _,
            } => {
                // TODO: We can use this to detect a dead peer
            }
            SSUOp::Report {
                op: SSUOpcode::Query,
                session_id: Some(_session_id),
                code: _,
            } => {
                // TODO: We can use this to detect a dead peer
            }
            SSUOp::Report {
                op: SSUOpcode::Reset,
                session_id: Some(_session_id),
                code: _,
            } => {
                // TODO: We can use this to detect a dead peer
            }
            SSUOp::Report {
                op: SSUOpcode::Select,
                session_id: Some(_session_id),
                code: _,
            } => {
                // TODO: We can use this to detect a dead peer
            }
            _ => {
                warn!("Ignored unhandled or invalid command: {:?}", op);
            }
        }
    }

    /// Once the peer's credits drops below the low water mark, we need to top them up
    /// with a constant amount.
    fn session_needs_credits(&self, session_id: u8) -> Option<usize> {
        let credits = self.credits[session_id as usize].get(CreditsSlot::PeerToSession);
        if credits < RECV_CREDITS_LOW_WATER_MARK {
            Some(RECV_CREDITS_TOP_UP)
        } else {
            None
        }
    }
}

pub enum ServerError {
    Closed,
}
