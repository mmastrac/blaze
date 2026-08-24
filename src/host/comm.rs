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
    /// Set once the endpoints are gone. Stops the polling and the logging.
    closed: bool,
}

impl CommSession {
    /// Log the disconnect once, then stop polling.
    fn note_error(&mut self, what: &str, e: &std::io::Error) {
        if e.kind() == std::io::ErrorKind::NotConnected {
            info!("Session disconnected, detaching from DUART channel");
            self.closed = true;
        } else {
            error!("Failed to {what} byte: {e}");
        }
    }

    pub fn tick(&mut self) {
        if self.closed {
            return;
        }

        // DUART's send to session's send
        let b = if let Some(pending) = self.pending_rx.take() {
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
                    self.note_error("send", &e);
                }
                Err(b) => {
                    self.pending_rx.replace(b);
                }
            }
        }

        // Session's recv to DUART's recv
        let b = if let Some(pending) = self.pending_tx.take() {
            Some(pending)
        } else {
            match self.session.try_recv() {
                Ok(Some(byte)) => Some(byte),
                Ok(None) => None,
                Err(e) => {
                    self.note_error("receive", &e);
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
        closed: false,
    })
}
