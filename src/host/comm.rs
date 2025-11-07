use std::cell::Cell;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;
use tracing::{error, trace};

use crate::machine::generic::duart::DUARTChannel;

/// Communication configuration for a DUART channel
#[derive(Debug, Clone, Default)]
pub enum CommConfig {
    /// Loopback mode (no external connection)
    #[default]
    Loopback,
    /// Single bidirectional pipe
    Pipe(PathBuf),
    /// Separate read and write pipes
    Pipes { rx: PathBuf, tx: PathBuf },
    /// Execute a command and connect to its pty
    Exec(String),
}

impl CommConfig {
    /// Parse command-line arguments into CommConfig
    pub fn from_args(
        pipe: Option<PathBuf>,
        pipes: Option<(PathBuf, PathBuf)>,
        exec: Option<String>,
    ) -> Self {
        if let Some(exec_cmd) = exec {
            CommConfig::Exec(exec_cmd)
        } else if let Some((rx, tx)) = pipes {
            CommConfig::Pipes { rx, tx }
        } else if let Some(pipe) = pipe {
            CommConfig::Pipe(pipe)
        } else {
            CommConfig::Loopback
        }
    }
}

/// Connect a DUART channel to the configured communication method
pub fn connect_duart(
    channel: DUARTChannel,
    config: CommConfig,
) -> Result<Rc<Cell<bool>>, std::io::Error> {
    match config {
        CommConfig::Loopback => connect_loopback(channel),
        CommConfig::Pipe(path) => connect_single_pipe(channel, path),
        CommConfig::Pipes { rx, tx } => connect_dual_pipes(channel, rx, tx),
        CommConfig::Exec(cmd) => connect_exec(channel, cmd),
    }
}

fn connect_loopback(channel: DUARTChannel) -> Result<Rc<Cell<bool>>, std::io::Error> {
    thread::spawn(move || {
        loop {
            match channel.rx.recv() {
                Ok(b) => {
                    trace!("DUART pipe loopback char {b:02X} {:?}", b as char);
                    if !channel.tx.send(b).is_ok() {
                        break;
                    }
                }
                _ => break,
            }
        }
        trace!("DUART pipe loopback thread exited");
    });
    Ok(channel.dtr)
}

fn connect_single_pipe(
    channel: DUARTChannel,
    path: PathBuf,
) -> Result<Rc<Cell<bool>>, std::io::Error> {
    let software_flow_control = Arc::new(AtomicBool::new(true));
    let rx = channel.rx;
    let tx = channel.tx;

    eprintln!("Opening {:?} as read/write", path);
    let mut pipe_r = OpenOptions::new().read(true).write(true).open(&path)?;
    let mut pipe_w = pipe_r.try_clone()?;
    eprintln!("Opened!");

    let software_flow_control_clone = software_flow_control.clone();
    thread::spawn(move || {
        loop {
            match rx.recv() {
                Ok(b) => {
                    if b == 0x11 {
                        // XON
                        trace!("DUART pipe XON");
                        software_flow_control_clone.store(true, Ordering::Relaxed);
                    } else if b == 0x13 {
                        // XOFF
                        trace!("DUART pipe XOFF");
                        software_flow_control_clone.store(false, Ordering::Relaxed);
                    } else {
                        if !pipe_w.write_all(&[b]).is_ok() {
                            break;
                        }
                    }
                }
                _ => break,
            }
        }
        trace!("DUART pipe write thread exited");
    });

    thread::spawn(move || {
        loop {
            if !software_flow_control.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            let mut buf = [0; 1];
            match pipe_r.read(&mut buf) {
                Ok(1) => {
                    if !tx.send(buf[0]).is_ok() {
                        break;
                    }
                }
                _ => break,
            }
        }
        trace!("DUART pipe read thread exited");
    });

    Ok(channel.dtr)
}

fn connect_dual_pipes(
    channel: DUARTChannel,
    pipe_r_path: PathBuf,
    pipe_w_path: PathBuf,
) -> Result<Rc<Cell<bool>>, std::io::Error> {
    let software_flow_control = Arc::new(AtomicBool::new(true));
    let rx = channel.rx;
    let tx = channel.tx;

    let software_flow_control_clone = software_flow_control.clone();
    thread::spawn(move || {
        let Ok(mut pipe_w) = OpenOptions::new().write(true).open(&pipe_w_path) else {
            error!("Failed to open pipe_w: {:?}", pipe_w_path);
            return;
        };
        loop {
            match rx.recv() {
                Ok(b) => {
                    if b == 0x11 {
                        // XON
                        trace!("DUART pipe XON");
                        software_flow_control_clone.store(true, Ordering::Relaxed);
                    } else if b == 0x13 {
                        // XOFF
                        trace!("DUART pipe XOFF");
                        software_flow_control_clone.store(false, Ordering::Relaxed);
                    } else {
                        if !pipe_w.write_all(&[b]).is_ok() {
                            break;
                        }
                    }
                }
                _ => break,
            }
        }
        trace!("DUART pipe write thread exited");
    });

    thread::spawn(move || {
        let Ok(mut pipe_r) = OpenOptions::new().read(true).open(&pipe_r_path) else {
            error!("Failed to open pipe_r: {:?}", pipe_r_path);
            return;
        };
        loop {
            if !software_flow_control.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            let mut buf = [0; 1];
            match pipe_r.read(&mut buf) {
                Ok(1) => {
                    if !tx.send(buf[0]).is_ok() {
                        break;
                    }
                }
                _ => break,
            }
        }
        trace!("DUART pipe read thread exited");
    });

    Ok(channel.dtr)
}

fn connect_exec(
    channel: DUARTChannel,
    cmd_string: String,
) -> Result<Rc<Cell<bool>>, std::io::Error> {
    use pty_process::blocking::Command;

    let software_flow_control = Arc::new(AtomicBool::new(true));
    let rx = channel.rx;
    let tx = channel.tx;

    if cmd_string.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Empty command string",
        ));
    }

    // Open PTY
    let (pty, pts) = pty_process::blocking::open()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    pty.resize(pty_process::Size::new(24, 80))
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    // Spawn command via shell
    let _child = Command::new("/bin/sh")
        .arg("-c")
        .arg(&cmd_string)
        .spawn(pts)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    let mut pty = File::from(OwnedFd::from(pty));
    let mut pty_read: File = pty.try_clone()?;

    let software_flow_control_clone = software_flow_control.clone();
    thread::spawn(move || {
        loop {
            match rx.recv() {
                Ok(b) => {
                    if b == 0x11 {
                        // XON
                        trace!("DUART pty XON");
                        software_flow_control_clone.store(true, Ordering::Relaxed);
                    } else if b == 0x13 {
                        // XOFF
                        trace!("DUART pty XOFF");
                        software_flow_control_clone.store(false, Ordering::Relaxed);
                    } else {
                        if !pty.write_all(&[b]).is_ok() {
                            break;
                        }
                    }
                }
                _ => break,
            }
        }
        trace!("DUART pty write thread exited");
    });

    thread::spawn(move || {
        loop {
            if !software_flow_control.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            let mut buf = [0; 1];
            let read_result = { pty_read.read(&mut buf) };
            match read_result {
                Ok(n) if n > 0 => {
                    if !tx.send(buf[0]).is_ok() {
                        break;
                    }
                }
                Ok(_) => break, // EOF (read 0 bytes)
                Err(_) => break,
            }
        }
        trace!("DUART pty read thread exited");
    });

    Ok(channel.dtr)
}
