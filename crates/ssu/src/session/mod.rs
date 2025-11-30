use std::{borrow::Cow, num::NonZeroU16, path::PathBuf, str::FromStr};

use clap::{CommandFactory, FromArgMatches, Parser};

#[cfg(feature = "pty")]
use crate::session::pty::PtySession;
use crate::session::{
    exec::ExecSession,
    io::{IoSessionReadWrite, boot_io},
    loopback::LoopbackSession,
    pipe::{DualPipeSession, SinglePipeSession},
};

pub mod exec;
pub mod io;
pub mod loopback;
pub mod pipe;
#[cfg(feature = "pty")]
pub mod pty;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionConfig {
    /// Loopback mode (no external connection)
    Loopback(String),
    /// Single bidirectional pipe
    Pipe(PathBuf),
    /// Separate read and write pipes
    Pipes { rx: PathBuf, tx: PathBuf },
    /// Execute a command and connect to its pty
    Exec(String),
    /// Execute a command and connect to its pty
    #[cfg(feature = "pty")]
    ExecPty {
        cmd: String,
        rows: NonZeroU16,
        cols: NonZeroU16,
    },
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self::Loopback(String::new())
    }
}

impl SessionConfig {
    pub fn start(self) -> std::io::Result<Box<dyn SessionEndpoint + Send + 'static>> {
        fn boot(
            session: impl IoSessionEndpoint,
        ) -> std::io::Result<Box<dyn SessionEndpoint + Send + 'static>> {
            boot_io(session).map(|io| Box::new(io) as _)
        }

        match self {
            SessionConfig::Loopback(initial) => Ok(Box::new(LoopbackSession::new(initial))),
            SessionConfig::Pipe(path) => boot(SinglePipeSession::new(path)),
            SessionConfig::Pipes { rx, tx } => boot(DualPipeSession::new(rx, tx)),
            SessionConfig::Exec(cmd) => boot(ExecSession::new(cmd)),
            #[cfg(feature = "pty")]
            SessionConfig::ExecPty { cmd, rows, cols } => boot(PtySession::new(cmd, cols, rows)),
        }
    }
}

/// Options for a session connection.
#[derive(clap::Parser, Debug)]
#[command(disable_help_flag = true, disable_version_flag = true)]
enum SessionSubcommand {
    /// Connect the read and write ends of the session to each other.
    Loopback {
        /// Provide initial text inserted into the loopback connection
        initial: Option<String>,
    },
    /// Connect the read and write ends of the session to separate pipes.
    Pipe {
        /// Provide a single pipe for both read and write.
        read_write: Option<PathBuf>,
        /// Provide a separate read pipe from the write pipe.
        read: Option<PathBuf>,
        /// Provide a separate write pipe from the read pipe.
        write: Option<PathBuf>,
    },
    /// Execute a command and connect to its pty or stdin/stdout.
    Exec {
        /// The command to execute.
        command: PathBuf,
        /// Allocate a PTY for the process.
        #[arg(long, default_value_t = false)]
        no_pty: bool,
        #[arg(long, default_value_t = NonZeroU16::new(24).unwrap(), conflicts_with = "no_pty")]
        rows: NonZeroU16,
        #[arg(long, default_value_t = NonZeroU16::new(80).unwrap(), conflicts_with = "no_pty")]
        cols: NonZeroU16,
    },
}

impl FromStr for SessionConfig {
    type Err = Cow<'static, str>;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();

        let mut args = shellish_parse::parse(s, shellish_parse::ParseOptions::new())
            .map_err(|_| "Invalid argument")?;
        // Prepend a dummy program name - clap expects the first argument to be the program name
        args.insert(0, "session".to_string());
        let command = SessionSubcommand::command()
            .name("<session>")
            .bin_name("<--arg>")
            .override_usage("<COMMAND>")
            .disable_colored_help(true)
            .disable_help_flag(true);
        let arg_matches = command
            .clone()
            .try_get_matches_from(args)
            .map_err(|s| format!("Invalid session configuration\n\n{}", s.with_cmd(&command)))?;
        let subcommand = SessionSubcommand::from_arg_matches(&arg_matches).map_err(|s| {
            format!(
                "Invalid session sc configuration\n\n{}",
                s.with_cmd(&command)
            )
        })?;
        Ok(match subcommand {
            SessionSubcommand::Loopback { initial } => {
                SessionConfig::Loopback(initial.unwrap_or_default())
            }
            SessionSubcommand::Exec {
                command,
                no_pty,
                rows,
                cols,
            } => {
                if !no_pty {
                    #[cfg(feature = "pty")]
                    {
                        return Ok(SessionConfig::ExecPty {
                            cmd: command.to_string_lossy().to_string(),
                            rows,
                            cols,
                        });
                    }
                    #[cfg(not(feature = "pty"))]
                    {
                        return Err(Cow::Borrowed(
                            "PTY mode not available (pty feature not enabled)",
                        ));
                    }
                } else {
                    SessionConfig::Exec(command.to_string_lossy().to_string())
                }
            }
            SessionSubcommand::Pipe {
                read_write,
                read,
                write,
            } => {
                if let Some(read_write) = read_write {
                    SessionConfig::Pipe(read_write)
                } else if let Some(read) = read {
                    SessionConfig::Pipes {
                        rx: read,
                        tx: write.unwrap(),
                    }
                } else {
                    unreachable!()
                }
            }
        })
    }
}

pub enum Ticked {
    /// Byte available.
    Byte(u8),
    /// Idle until the next `send` call.
    IdleInput,
    /// Idle right now, try again later.
    Idle,
}

/// A session endpoint that can be ticked. Backpressure is applied by the
/// caller.
pub trait SessionEndpoint {
    fn recv(&mut self) -> Ticked;
    fn send(&mut self, b: u8);

    fn split(
        self: Box<Self>,
    ) -> (
        Box<dyn SessionRecvEndpoint + Send + 'static>,
        Box<dyn SessionSendEndpoint + Send + 'static>,
    );
}

pub trait SessionRecvEndpoint {
    fn recv(&mut self) -> Ticked;
}

pub trait SessionSendEndpoint {
    fn send(&mut self, b: u8);
}

pub struct DynSessionEndpoint {
    endpoint: Box<dyn SessionEndpoint>,
}

/// A I/O-based session endpoint that is started in a separate thread.
/// Backpressure is applied by SyncSender filling up at which point the endpoint
/// should apply hardware flow control signals if available.
pub trait IoSessionEndpoint {
    /// Start the session endpoint in a separate thread.
    fn start(self, ready: impl FnOnce(std::io::Result<IoSessionReadWrite>) + Send + 'static);
}
