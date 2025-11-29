use std::cell::{Cell, RefCell};
use std::future::poll_fn;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Poll, Waker};
use std::time::Duration;
use std::{array, io, slice, thread};

use tracing::{info, trace};

use crate::{INTRO, OP_ADDCR, OP_PROBE, OP_SELECT, TERM};

/// The maximum number of bytes to send in a single chunk before
/// trying to poll the other session.
const CHUNK_SIZE: usize = 32;

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
const PEER_COMMAND_QUEUE_SIZE: usize = 256;

const MAX_SESSION_COUNT: usize = 4;

/// SSU server state machine implementation. This speaks SSU to a peer and
/// provides multiple SSU endpoints as sessions.
///
/// Data is both pushed and pulled externally for the peer and individual
/// sessions, while this server manages the credits and routing for data.
pub struct Server {
    active_session: Option<u8>,
    outgoing_command_queue: Arc<Mutex<SyncRingBuffer<PEER_COMMAND_QUEUE_SIZE>>>,
    select_command: Arc<Mutex<SyncRingBuffer<4>>>,
    xon: Xon,
    sessions: [Session; MAX_SESSION_COUNT],
    max_sessions: u8,
    /// If we get stuck without credits for any session, we may need to defer
    /// processing until we get some room.
    stuck: Arc<WakerHandle>,
}

impl Server {
    pub fn new(max_sessions: u8) -> ServerHandles {
        let server = Server {
            sessions: Default::default(),
            outgoing_command_queue: Default::default(),
            select_command: Default::default(),
            xon: Default::default(),
            active_session: Default::default(),
            stuck: Default::default(),
            max_sessions,
        };
        send_probe_message(
            &mut *server.outgoing_command_queue.lock().unwrap(),
            0,
            0,
            max_sessions,
        );

        let server = ServerHandle {
            server: Arc::new(Mutex::new(server)),
        };

        let server_read = {
            let server = server.clone();
            let lock = server.lock();
            let outgoing_command_queue = lock.outgoing_command_queue.clone();
            let select_buffer = lock.select_command.clone();
            drop(lock);
            ServerRead {
                outgoing_command_queue,
                select_buffer,
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
                    server: server.clone(),
                },
            )
        });

        ServerHandles {
            server_read,
            server_write: ServerWrite {
                server: server.clone(),
                incoming_command_queue: Default::default(),
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
}

#[derive(Default)]
pub struct Session {
    send: RingBufferHandle<SESSION_TO_PEER_SIZE>,
    recv: RingBufferHandle<PEER_TO_SESSION_SIZE>,
    credits: Credits,
    peer_credits: usize,
}

fn send_probe_message<const SIZE: usize>(
    queue: &mut SyncRingBuffer<SIZE>,
    state: u8,
    protocol_variant: u8,
    max_sessions: u8,
) {
    queue.push(INTRO);
    queue.push(OP_PROBE);
    queue.push(b'A' + state);
    queue.push(b'A' + protocol_variant);
    queue.push(b'A' + max_sessions - 1);
    queue.push(TERM);
}

fn send_select_message<const SIZE: usize>(queue: &mut SyncRingBuffer<SIZE>, session_id: u8) {
    queue.push(INTRO);
    queue.push(OP_SELECT);
    queue.push(b'A' + session_id);
    queue.push(TERM);
}

fn send_add_credits_message<const SIZE: usize>(
    queue: &mut SyncRingBuffer<SIZE>,
    session_id: u8,
    credits: usize,
) {
    queue.push(INTRO);
    queue.push(OP_ADDCR);
    queue.push(b'A' + session_id);

    // Credits = { z5, x4, x3, x2, x1, x0, y4, y3, y2, y1, y0, z4, z3, z2, z1, z0 }

    let total = credits as u16;
    let x = ((total >> 10) & 0x1F) as u8;
    let y = ((total >> 5) & 0x1F) as u8;
    let mut z = (total & 0x1F) as u8;

    if (total & 0x8000) != 0 {
        z |= 0x20; // set z5 (bit5 of z byte)
    }

    queue.push(x + b'@');
    queue.push(y + b'@');
    queue.push(z + b'@');
    queue.push(TERM);
}

/// Run the server against a set of session endpoints and a peer.
// pub fn run(
//     sessions: Vec<Box<dyn SessionEndpoint + Send + 'static>>,
//     peer_in: impl io::Read,
//     peer_out: impl io::Write,
// ) -> Result<(), io::Error> {
//     let server = Server::new(sessions.len() as u8);

//     for (session, server_session) in sessions.into_iter().zip(&server.sessions) {
//         let (mut recv, mut send) = session.split();
//         thread::spawn(move || {
//             loop {
//                 match recv.recv() {
//                     _ => {}
//                 }
//             }
//         });
//         thread::spawn(move || {
//             loop {
//                 _ = &send;
//             }
//         });
//     }

//     loop {}
// }

#[cfg(feature = "server")]
pub async fn run_async() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    trace!("Entering run_async");
    let server = Server::new(2);
    let mut server_read = server.server_read;
    let mut stdout = tokio::io::stdout();
    tokio::task::spawn(async move {
        trace!("Entering stdout");
        loop {
            let Ok(b) = server_read.read().await else {
                return;
            };
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
            }
        });
        tokio::task::spawn(async move {
            loop {
                for b in b"Hello world\n" {
                    let Ok(()) = write.write(*b).await else {
                        return;
                    };
                }
            }
        });
    }

    loop {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[derive(Default)]
struct WakerHandle {
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

    pub fn wait(&self) -> impl Future<Output = ()> {
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

#[derive(Default)]
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

struct SyncRingBuffer<const SIZE: usize> {
    buffer: [u8; SIZE],
    write_index: usize,
    read_index: usize,
}

impl<const SIZE: usize> Default for SyncRingBuffer<SIZE> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const SIZE: usize> SyncRingBuffer<SIZE> {
    pub fn new() -> Self {
        Self {
            buffer: [0; SIZE],
            write_index: 0,
            read_index: 0,
        }
    }

    pub fn push(&mut self, b: u8) {
        assert!(!self.is_full());
        self.buffer[self.write_index] = b;
        self.write_index = (self.write_index + 1) % SIZE;
    }

    pub fn pop(&mut self) -> Option<u8> {
        if self.read_index == self.write_index {
            return None;
        }
        let b = self.buffer[self.read_index];
        self.read_index = (self.read_index + 1) % SIZE;
        Some(b)
    }

    pub fn is_empty(&self) -> bool {
        self.read_index == self.write_index
    }

    pub fn is_full(&self) -> bool {
        (self.write_index + 1) % SIZE == self.read_index
    }

    pub fn clear(&mut self) {
        self.read_index = self.write_index;
    }

    pub fn len(&self) -> usize {
        if self.write_index >= self.read_index {
            self.write_index - self.read_index
        } else {
            SIZE - self.read_index + self.write_index
        }
    }
}

struct RingBuffer<const SIZE: usize> {
    buffer: [u8; SIZE],
    write_waker: Arc<WakerHandle>,
    read_waker: Arc<WakerHandle>,
    write_index: usize,
    read_index: usize,
}

impl<const SIZE: usize> Default for RingBuffer<SIZE> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const SIZE: usize> RingBuffer<SIZE> {
    pub fn new() -> Self {
        Self {
            buffer: [0; SIZE],
            write_waker: Arc::new(WakerHandle::new()),
            read_waker: Arc::new(WakerHandle::new()),
            write_index: 0,
            read_index: 0,
        }
    }

    /// Check if the buffer is full
    fn is_full(&self) -> bool {
        (self.write_index + 1) % self.buffer.len() == self.read_index
    }

    /// Check if the buffer is empty
    fn is_empty(&self) -> bool {
        self.read_index == self.write_index
    }

    pub async fn push(this: &Arc<Mutex<Self>>, b: u8) {
        // Wait if buffer is full
        loop {
            let waker = {
                let mut lock = this.lock().unwrap();
                if !lock.is_full() {
                    let write_index = lock.write_index;
                    lock.buffer[write_index] = b;
                    lock.write_index = (lock.write_index + 1) % lock.buffer.len();

                    // Wake any waiting readers
                    lock.read_waker.maybe_wake();
                    return;
                }
                lock.write_waker.clone()
            };
            waker.wait().await;
        }
    }

    pub async fn pop(this: &Arc<Mutex<Self>>) -> u8 {
        // Wait if buffer is empty
        loop {
            let waker = {
                let mut lock = this.lock().unwrap();
                if !lock.is_empty() {
                    // Pop the byte
                    let b = lock.buffer[lock.read_index];
                    lock.read_index = (lock.read_index + 1) % lock.buffer.len();

                    // Wake any waiting writers
                    lock.write_waker.maybe_wake();

                    return b;
                }
                lock.read_waker.clone()
            };
            waker.wait().await;
        }
    }

    pub fn pop_sync(&mut self) -> Option<u8> {
        if self.is_empty() {
            return None;
        }
        let b = self.buffer[self.read_index];
        self.read_index = (self.read_index + 1) % self.buffer.len();
        Some(b)
    }
}

#[derive(Clone, Default)]
pub struct RingBufferHandle<const SIZE: usize> {
    buffer: Arc<Mutex<RingBuffer<SIZE>>>,
}

impl<const SIZE: usize> RingBufferHandle<SIZE> {
    pub fn new() -> Self {
        Self {
            buffer: Arc::new(Mutex::new(RingBuffer::new())),
        }
    }

    pub async fn push(&self, b: u8) {
        RingBuffer::push(&self.buffer, b).await;
    }

    pub async fn pop(&self) -> u8 {
        RingBuffer::pop(&self.buffer).await
    }

    pub fn pop_sync(&self) -> Option<u8> {
        let mut lock = self.buffer.lock().unwrap();
        lock.pop_sync()
    }
}

pub struct SessionRead {
    buffer: RingBufferHandle<PEER_TO_SESSION_SIZE>,
    server: ServerHandle,
}

impl SessionRead {
    /// Reads a byte from the peer to send to the session.
    pub async fn read(&mut self) -> Result<u8, SessionError> {
        Ok(self.buffer.pop().await)
    }
}

pub struct SessionWrite {
    buffer: RingBufferHandle<SESSION_TO_PEER_SIZE>,
    server: ServerHandle,
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
    outgoing_command_queue: Arc<Mutex<SyncRingBuffer<PEER_COMMAND_QUEUE_SIZE>>>,
    select_buffer: Arc<Mutex<SyncRingBuffer<4>>>,
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
        self.server.wait_xon().await;
        if let Some(b) = self.outgoing_command_queue.lock().unwrap().pop() {
            trace!("Sending command byte: {b:02X}");
            return Ok(b);
        }
        trace!("No command bytes to send");
        loop {
            for i in 0..self.server.lock().max_sessions {
                let session = &self.server.lock().sessions[i as usize];
                let recv = session.recv.clone();
                if let Some(b) = recv.pop_sync() {
                    trace!("Sending session {i} byte: {b:02X}");
                    return Ok(b);
                }
            }

            let stuck = self.server.lock().stuck.clone();
            stuck.wait().await;
        }
    }
}

pub struct ServerWrite {
    incoming_command_queue: SyncRingBuffer<PEER_COMMAND_QUEUE_SIZE>,
    server: ServerHandle,
}

impl ServerWrite {
    /// Writes a byte from the peer. If the internal buffers for the sessions
    /// are full, will be in the pending state.
    pub async fn write(&mut self, b: u8) -> Result<(), ServerError> {
        trace!("Writing byte {b:02X}");
        match b {
            0x11 => {
                self.server.set_xon(true);
            }
            0x13 => {
                self.server.set_xon(false);
            }
            0x3 | 0x4 => {
                // todo: ctrl+c or ctrl+d exits the server for now
                std::process::exit(1);
            }
            INTRO => {
                self.incoming_command_queue.push(INTRO);
            }
            TERM => {
                let server = self.server.lock();
                if !self.incoming_command_queue.is_empty() {
                    // If the command queue is full, we overflowed
                    if !self.incoming_command_queue.is_full() {
                        // todo
                    }
                    self.incoming_command_queue.clear();
                }
                // The command might have unstuck us, so let's try to wake up
                // the peer reader.
                server.stuck.maybe_wake();
            }
            _ => {
                if !self.incoming_command_queue.is_empty() {
                    self.incoming_command_queue.push(b);
                } else {
                    let active_session = self.server.lock().active_session;
                    if let Some(active) = active_session {
                        let recv = {
                            let session = &self.server.lock().sessions[active as usize];
                            session.recv.clone()
                        };
                        recv.push(b).await;
                    } else {
                        // These bytes are unallocated and go to the bit bucket
                    }
                }
            }
        }
        Ok(())
    }
}

pub enum ServerError {
    Closed,
}
