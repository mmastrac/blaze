use std::array;
use std::future::{Future, poll_fn};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Poll, Waker};
use std::time::Duration;

use tracing::{error, info, trace, warn};

use crate::buffer::{RingBufferHandle, SyncRingBuffer};
use crate::ops::{
    INTRO, MAX_COMMAND_LEN, MAX_LABEL_LEN, OP_OPEN, OP_REQUEST_RESTORE, OP_RESTORE, OP_RESTORE_END,
    OP_SELECT, OP_VERIFY, SSUOp, SSUOpcode, SSUState, SSUString, TERM,
};

/// The maximum number of bytes to send in a single chunk before
/// trying to poll the other session.
const FAIRNESS_CHUNK_SIZE: usize = 32;

const RECV_CREDITS_LOW_WATER_MARK: usize = 128;
const RECV_CREDITS_TOP_UP: usize = 1024;

/// The maximum internal buffer size to receive from a session
/// regardless of upstream credit count.
const SESSION_TO_PEER_SIZE: usize = 1024;
/// The maximum internal buffer size to queue for a session. This affects
/// the credit messages sent to the upstream peer and is effectively what
/// we promise to buffer per session to the upstream peer.
const PEER_TO_SESSION_SIZE: usize = 16 * 1024;

/// The maximum internal buffer size to queue for the peer's command queue.
const PEER_COMMAND_QUEUE_SIZE: usize = 32;

const MAX_SESSION_COUNT: usize = 4;

/// SSU server state machine implementation. This speaks SSU to a peer and
/// provides multiple SSU endpoints as sessions.
///
/// Data is both pushed and pulled externally for the peer and individual
/// sessions, while this server manages the credits and routing for data.
pub struct Server {
    active_session_from_peer: Option<u8>,
    /// The command queue for the peer.
    xon: Xon,
    sessions: [Session; MAX_SESSION_COUNT],
    max_sessions: u8,
    /// If we get stuck without credits for any session, we may need to defer
    /// processing until we get some room.
    stuck: Arc<WakerHandle>,
}

impl Server {
    pub fn new(max_sessions: u8) -> ServerHandles {
        // This is shared by a number of actors in the system
        let outgoing_command_queue = RingBufferHandle::default();
        outgoing_command_queue.push_sync(SSUOp::Probe(SSUState::Disabled, 1, max_sessions));

        let server = Server {
            sessions: Default::default(),
            xon: Xon::new(),
            active_session_from_peer: Default::default(),
            stuck: Default::default(),
            max_sessions,
        };

        let server = ServerHandle {
            server: Arc::new(Mutex::new(server)),
        };

        let server_read = {
            let server = server.clone();
            let lock = server.lock();
            drop(lock);
            ServerRead {
                active_session_to_peer: None,
                outgoing_command_queue: outgoing_command_queue.clone(),
                outgoing_command_queue_bytes: Default::default(),
                server,
                fairness_counter: 0,
            }
        };

        let sessions = array::from_fn(|_| {
            (
                SessionRead {
                    buffer: Default::default(),
                    server: server.clone(),
                },
                SessionWrite {
                    buffer: Default::default(),
                },
            )
        });

        ServerHandles {
            server_read,
            server_write: ServerWrite {
                server: server.clone(),
                incoming_command_queue: Default::default(),
                outgoing_command_queue,
            },
            session_handles: SessionHandles { sessions, count: 0 },
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

    pub fn active_session_recv(&self) -> Option<RingBufferHandle<PEER_TO_SESSION_SIZE, u8>> {
        let server = self.lock();
        if let Some(active) = server.active_session_from_peer {
            Some(server.sessions[active as usize].recv.clone())
        } else {
            None
        }
    }

    pub fn try_recv(&self, session_id: u8) -> RecvResult {
        let mut server = self.lock();
        let session = &mut server.sessions[session_id as usize];
        if session.recv.is_empty() {
            return RecvResult::NoData;
        }
        if !session.credits.try_take_one().is_ok() {
            trace!("No credits remaining for session {session_id}");
            return RecvResult::NoCredits;
        }
        trace!("Credits remaining: {}", session.credits.count);
        match session.recv.pop_sync() {
            Some(b) => RecvResult::Data(b),
            None => RecvResult::NoData,
        }
    }

    fn lock(&self) -> MutexGuard<Server> {
        self.server.lock().unwrap()
    }

    async fn await_stuck(&self) {
        let stuck = self.lock().stuck.clone();
        stuck.wait().await;
    }

    fn max_session(&self) -> u8 {
        self.lock().max_sessions
    }
}

#[derive(Default)]
pub struct Session {
    send: RingBufferHandle<SESSION_TO_PEER_SIZE, u8>,
    recv: RingBufferHandle<PEER_TO_SESSION_SIZE, u8>,
    credits: Credits,
    peer_credits: usize,
}

#[cfg(feature = "server")]
pub async fn run_async() {
    use std::os::fd::AsFd;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    trace!("Entering run_async");
    let server = Server::new(2);
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

    for (mut read, mut write) in server.session_handles.into_iter() {
        tokio::task::spawn(async move {
            loop {
                let Ok(b) = read.read().await else {
                    return;
                };
                trace!("Got byte {b:02X} from session, looping back");
                let Ok(()) = write.write(b).await else {
                    return;
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
            } else {
                Poll::Ready(())
            }
        })
    }

    pub fn maybe_wake(&self) {
        if let Some(waker) = self.waker.lock().unwrap().take() {
            waker.wake();
        }
    }

    pub fn must_wake(&self) {
        assert!(self.waker.lock().unwrap().is_some());
        self.maybe_wake();
    }
}

#[derive(Default)]
struct Credits {
    count: usize,
    waker: WakerHandle,
}

impl Credits {
    pub fn new() -> Self {
        Self {
            count: 0,
            waker: WakerHandle::new(),
        }
    }

    pub fn add(&mut self, count: usize) {
        self.count = self.count.saturating_add(count);
        self.waker.maybe_wake();
    }

    pub fn zero(&mut self) {
        self.count = 0;
    }

    pub fn try_take_one(&mut self) -> Result<(), ()> {
        if self.count == 0 {
            return Err(());
        }
        self.count = self.count.saturating_sub(1);
        Ok(())
    }

    pub async fn take_one(&mut self) {
        loop {
            if self.count == 0 {
                self.waker.wait().await;
                continue;
            }

            self.count = self.count.saturating_sub(1);
            return;
        }
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
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

    pub async fn check(&self) {
        if !self.xon {
            self.waker.wait().await;
        }
    }
}

pub struct SessionRead {
    buffer: RingBufferHandle<PEER_TO_SESSION_SIZE, u8>,
    server: ServerHandle,
}

impl SessionRead {
    /// Reads a byte from the peer to send to the session.
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
    active_session_to_peer: Option<u8>,
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

            // There are no outgoing commands, so check for data. This is
            // curently not a fully-optimal or fair algorithm.
            trace!("Checking sessions");

            // First, check the active session. We'll read up to
            // FAIRNESS_CHUNK_SIZE bytes from the active session before giving
            // another session a chance.
            if self.fairness_counter > 0 {
                if let Some(active) = self.active_session_to_peer {
                    self.fairness_counter = self.fairness_counter.saturating_sub(1);
                    match self.server.try_recv(active) {
                        RecvResult::Data(b) => return Ok(b),
                        RecvResult::NoData => {}
                        RecvResult::NoCredits => {
                            // We should not hit this case because the remote
                            // end should have granted us more, but it's
                            // possible we've lost data on the line somewhere.
                            let mut buf = [0; MAX_COMMAND_LEN];
                            self.outgoing_command_queue_bytes.replace_with_slice(
                                SSUOp::<0>::Verify(active).serialize(&mut buf).unwrap(),
                            );
                            continue 'read;
                        }
                    }
                }
            }

            // If the action session is idle, we'll check the other sessions. To
            // be fair here we should keep some sort of LRU list.
            for i in 0..self.server.max_session() {
                match self.server.try_recv(i) {
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
                            // We already have this byte, so send it as part of the command queue
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
                        self.outgoing_command_queue_bytes
                            .replace_with_slice(SSUOp::<0>::Verify(i).serialize(&mut buf).unwrap());
                        continue 'read;
                    }
                }
            }

            trace!("Stuck");
            self.server.await_stuck().await;
        }
    }
}

pub struct ServerWrite {
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
            0x3 => {
                // todo: ctrl+c or ctrl+d exits the server for now
                std::process::exit(1);
            }
            0x4 => {
                let active = self.server.lock().active_session_from_peer.unwrap();
                self.outgoing_command_queue
                    .push(SSUOp::Verify(active))
                    .await;
                self.server.lock().stuck.maybe_wake();
            }
            0x5 => {
                let active = self.server.lock().active_session_from_peer.unwrap();
                self.outgoing_command_queue.push(SSUOp::Reset(active)).await;
                self.server.lock().stuck.maybe_wake();
            }
            0x6 => {
                let active = self.server.lock().active_session_from_peer.unwrap();
                self.outgoing_command_queue
                    .push(SSUOp::Close(active, false))
                    .await;
                self.server.lock().stuck.maybe_wake();
                let active = self.server.lock().active_session_from_peer.unwrap();
                self.outgoing_command_queue.push(SSUOp::Query(active)).await;
                self.server.lock().stuck.maybe_wake();
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
                            "Parsed command: {:?} from {:?}",
                            op,
                            String::from_utf8_lossy(op_buf)
                        );
                        if let Ok(op) = op {
                            self.process_op(op).await;
                        }
                    }
                    self.incoming_command_queue.clear();
                }
                // The command might have unstuck us, so let's try to wake up
                // the peer reader.
                self.server.lock().stuck.maybe_wake();
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
                } else {
                    if let Some(active) = self.server.active_session_recv() {
                        trace!("Pushing byte {b:02X} to active session");
                        active.push(b).await;
                        self.server.lock().stuck.maybe_wake();
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
            SSUOp::Probe(_state, protocol_variant, max_sessions) => {
                let max_sessions = max_sessions.max(self.server.max_session());
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
            }
            SSUOp::Select(session_id) => {
                self.server.lock().active_session_from_peer = Some(session_id);
                self.outgoing_command_queue
                    .push(SSUOp::Report {
                        op: SSUOpcode::Select,
                        session_id: Some(session_id),
                        code: 0,
                    })
                    .await;
                self.outgoing_command_queue
                    .push(SSUOp::AddCredits {
                        session_id,
                        credits: RECV_CREDITS_TOP_UP,
                    })
                    .await;
            }
            SSUOp::AddCredits {
                session_id,
                credits,
            } => {
                self.server.lock().sessions[session_id as usize]
                    .credits
                    .add(credits);
            }
            SSUOp::Zero(session_id) => {
                self.server.lock().sessions[session_id as usize]
                    .credits
                    .zero();
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
            SSUOp::Verify(session_id) => {
                self.outgoing_command_queue
                    .push(SSUOp::Report {
                        op: SSUOpcode::Verify,
                        session_id: None,
                        code: 0,
                    })
                    .await;
                self.outgoing_command_queue
                    .push(SSUOp::AddCredits {
                        session_id,
                        credits: RECV_CREDITS_TOP_UP,
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
                code,
            } => {
                // TODO: We can use this to detect a dead peer
            }
            SSUOp::Report {
                op: SSUOpcode::Restore,
                session_id: None,
                code,
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
                code,
            } => {
                self.outgoing_command_queue
                    .push(SSUOp::AddCredits {
                        session_id,
                        credits: RECV_CREDITS_TOP_UP,
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
                session_id: Some(session_id),
                code,
            } => {
                // TODO: We can use this to detect a dead peer
            }
            SSUOp::Report {
                op: SSUOpcode::RestoreEnd,
                session_id: None,
                code,
            } => {
                // TODO: We can use this to detect a dead peer
            }
            SSUOp::Report {
                op: SSUOpcode::Verify,
                session_id: Some(session_id),
                code,
            } => {
                // TODO: We can use this to detect a dead peer
            }
            SSUOp::Report {
                op: SSUOpcode::Query,
                session_id: Some(session_id),
                code,
            } => {
                // TODO: We can use this to detect a dead peer
            }
            SSUOp::Report {
                op: SSUOpcode::Reset,
                session_id: Some(session_id),
                code,
            } => {
                // TODO: We can use this to detect a dead peer
            }
            SSUOp::Report {
                op: SSUOpcode::Select,
                session_id: Some(_session_id),
                code,
            } => {
                // TODO: We can use this to detect a dead peer
            }
            _ => {
                warn!("Ignored unhandled or invalid command: {:?}", op);
            }
        }
    }
}

pub enum ServerError {
    Closed,
}
