use std::task::{Context, Poll};
use std::{fmt, io};

mod config;
mod config_parser;
mod io_session;
mod sync;

pub use config::SessionConfig;
pub use sync::SyncSession;

pub mod exec;
pub mod loopback;
pub mod pipe;
#[cfg(feature = "pty")]
pub mod pty;
#[cfg(feature = "serial")]
pub mod serial;
#[cfg(feature = "wasm")]
pub mod wasm;
pub mod xonoff;

/// Return a result or a "come back later". This will be replaced with `async`
/// in the future.
pub enum Ticked {
    /// Byte available.
    Byte(u8),
    /// Idle right now, try again later.
    Idle,
}

pub struct SessionParts {
    pub send: Box<dyn SessionSendEndpoint + Send + 'static>,
    pub recv: Box<dyn SessionRecvEndpoint + Send + 'static>,
}

impl SessionParts {
    pub fn new(
        send: impl SessionSendEndpoint + Send + 'static,
        recv: impl SessionRecvEndpoint + Send + 'static,
    ) -> Self {
        Self {
            send: Box::new(send),
            recv: Box::new(recv),
        }
    }
}

pub struct SessionPartsUnsend {
    pub send: Box<dyn SessionSendEndpoint + 'static>,
    pub recv: Box<dyn SessionRecvEndpoint + 'static>,
}

impl From<SessionParts> for SessionPartsUnsend {
    fn from(parts: SessionParts) -> Self {
        Self {
            send: parts.send,
            recv: parts.recv,
        }
    }
}

impl SessionPartsUnsend {
    pub fn new(
        send: impl SessionSendEndpoint + 'static,
        recv: impl SessionRecvEndpoint + 'static,
    ) -> Self {
        Self {
            send: Box::new(send),
            recv: Box::new(recv),
        }
    }
}

pub trait SessionRecvEndpoint: fmt::Debug {
    fn poll_recv(&mut self, ctx: &mut Context<'_>) -> Poll<io::Result<u8>>;
}

impl SessionRecvEndpoint for Box<dyn SessionRecvEndpoint + 'static> {
    fn poll_recv(&mut self, ctx: &mut Context<'_>) -> Poll<io::Result<u8>> {
        self.as_mut().poll_recv(ctx)
    }
}

impl SessionRecvEndpoint for Box<dyn SessionRecvEndpoint + Send + 'static> {
    fn poll_recv(&mut self, ctx: &mut Context<'_>) -> Poll<io::Result<u8>> {
        self.as_mut().poll_recv(ctx)
    }
}

pub trait SessionSendEndpoint: fmt::Debug {
    fn poll_send(&mut self, ctx: &mut Context<'_>, b: u8) -> Poll<io::Result<()>>;
}

impl SessionSendEndpoint for Box<dyn SessionSendEndpoint + 'static> {
    fn poll_send(&mut self, ctx: &mut Context<'_>, b: u8) -> Poll<io::Result<()>> {
        self.as_mut().poll_send(ctx, b)
    }
}

impl SessionSendEndpoint for Box<dyn SessionSendEndpoint + Send + 'static> {
    fn poll_send(&mut self, ctx: &mut Context<'_>, b: u8) -> Poll<io::Result<()>> {
        self.as_mut().poll_send(ctx, b)
    }
}

pub trait Session {
    fn boot(self) -> std::io::Result<SessionParts>;
}

pub trait SessionUnsend {
    fn boot(self) -> std::io::Result<SessionPartsUnsend>;
}
