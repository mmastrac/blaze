use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};
use std::thread;
use std::time::Duration;

use ssu::session::Ticked;
use ssu::session::loopback::LoopbackSession;
use ssu::session::pipe::{DualPipeSession, SinglePipeSession};
#[cfg(feature = "pty")]
use ssu::session::pty::PtySession;
#[cfg(feature = "serial")]
use ssu::session::serial::SerialSession;
#[cfg(feature = "wasm")]
use ssu::session::wasm::WasmSession;
use ssu::session::{IoSessionEndpoint, SessionConfig, SessionEndpoint, exec::ExecSession};

use crate::machine::generic::duart::DUARTChannel;

use tracing::{error, info};

pub enum CommSession {
    Tickable(
        Box<dyn SessionEndpoint>,
        Receiver<u8>,
        SyncSender<u8>,
        Option<u8>,
        bool,
    ),
    Io,
}

impl CommSession {
    pub fn tick(&mut self) {
        match self {
            CommSession::Tickable(session, rx, tx, pending, xon) => {
                match rx.try_recv() {
                    Ok(0x11) => {
                        info!("XON received");
                        *xon = true;
                    }
                    Ok(0x13) => {
                        info!("XOFF received");
                        *xon = false;
                    }
                    Ok(byte) => {
                        session.send(byte);
                    }
                    Err(e) => {}
                };

                if *xon {
                    let b = if let Some(pending) = pending.take() {
                        Some(pending)
                    } else {
                        match session.recv() {
                            Ticked::Byte(byte) => Some(byte),
                            Ticked::IdleInput => None,
                            Ticked::Idle => None,
                        }
                    };
                    if let Some(byte) = b {
                        match tx.try_send(byte) {
                            Ok(()) => {}
                            Err(TrySendError::Full(byte)) => {
                                pending.replace(byte);
                            }
                            Err(TrySendError::Disconnected(_)) => {}
                        }
                    }
                }
            }
            CommSession::Io => {}
        }
    }
}

fn boot_io(
    channel: DUARTChannel,
    io: impl IoSessionEndpoint,
) -> Result<CommSession, std::io::Error> {
    io.start(|session| {
        match session {
            Ok(session) => {
                let mut reader = session.reader;
                let mut writer = session.writer;
                let rx = channel.rx;
                let tx = channel.tx;
                let xoff = Arc::new(AtomicBool::new(false));

                let xoff_clone = xoff.clone();
                thread::spawn(move || {
                    let mut buf = [0; 1];
                    loop {
                        match reader.read_exact(&mut buf) {
                            Ok(_) => {}
                            Err(e) => {
                                error!("Failed to read byte from RX: {}", e);
                                break;
                            }
                        }
                        while xoff_clone.load(Ordering::Relaxed) {
                            // Spin wait for XOFF to be cleared
                            thread::sleep(Duration::from_millis(10));
                        }
                        match tx.send(buf[0]) {
                            Ok(()) => {}
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
                            Ok(0x13) => {
                                xoff.store(true, Ordering::Relaxed);
                            }
                            Ok(0x11) => {
                                xoff.store(false, Ordering::Relaxed);
                            }
                            Ok(byte) => {
                                writer.write_all(&[byte]).unwrap();
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
    });
    Ok(CommSession::Io)
}

/// Connect a DUART channel to the configured communication method
pub fn connect_duart(
    channel: DUARTChannel,
    config: SessionConfig,
) -> Result<CommSession, std::io::Error> {
    Ok(match config {
        SessionConfig::Loopback(initial) => CommSession::Tickable(
            Box::new(LoopbackSession::new(initial)),
            channel.rx,
            channel.tx,
            None,
            false,
        ),
        SessionConfig::Pipe(path) => boot_io(channel, SinglePipeSession::new(path))?,
        SessionConfig::Pipes { rx, tx } => boot_io(channel, DualPipeSession::new(rx, tx))?,
        SessionConfig::Exec(cmd) => boot_io(channel, ExecSession::new(cmd))?,
        #[cfg(feature = "pty")]
        SessionConfig::ExecPty { cmd, rows, cols } => {
            boot_io(channel, PtySession::new(cmd, cols, rows))?
        }
        #[cfg(feature = "serial")]
        SessionConfig::Serial {
            path,
            baud_rate,
            data_bits,
            stop_bits,
            flow_control,
        } => boot_io(
            channel,
            SerialSession::new(path, baud_rate, data_bits, stop_bits, flow_control),
        )?,
        #[cfg(feature = "wasm")]
        SessionConfig::Wasm { read_fn, write_fn } => CommSession::Tickable(
            Box::new(WasmSession::new(read_fn, write_fn)?),
            channel.rx,
            channel.tx,
            None,
            false,
        ),
        #[cfg(feature = "wasm")]
        SessionConfig::MessageChannel {} => CommSession::Tickable(
            Box::new(WasmSession::new_message_channel()?),
            channel.rx,
            channel.tx,
            None,
            false,
        ),
    })
}
