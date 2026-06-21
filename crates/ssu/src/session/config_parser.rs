use std::borrow::Cow;
#[cfg(feature = "pty")]
use std::num::NonZeroU16;
#[cfg(feature = "serial")]
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::str::FromStr;

use clap::{CommandFactory, FromArgMatches};

use crate::session::config::SessionConfig;
use crate::session::exec::ExecConfig;
use crate::session::loopback::LoopbackConfig;
use crate::session::pipe::{PipeConfig, PipesConfig};
#[cfg(feature = "pty")]
use crate::session::pty::ExecPtyConfig;
#[cfg(feature = "serial")]
use crate::session::serial::{SerialConfig, SerialFlowControl};
#[cfg(feature = "wasm")]
use crate::session::wasm::{MessageChannelConfig, WasmConfig};

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
        #[cfg(feature = "pty")]
        #[arg(long, default_value_t = NonZeroU16::new(24).unwrap(), conflicts_with = "no_pty")]
        rows: NonZeroU16,
        #[cfg(feature = "pty")]
        #[arg(long, default_value_t = NonZeroU16::new(80).unwrap(), conflicts_with = "no_pty")]
        cols: NonZeroU16,
    },
    /// Connect to a serial port
    #[cfg(feature = "serial")]
    Serial {
        path: PathBuf,
        #[arg(long, default_value_t = NonZeroU32::new(38400).unwrap())]
        baud_rate: NonZeroU32,
        #[arg(long, default_value_t = 8)]
        data_bits: u8,
        #[arg(long, default_value_t = 1)]
        stop_bits: u8,
        #[arg(
            long,
            default_value_t = false,
            conflicts_with = "software_flow_control"
        )]
        hardware_flow_control: bool,
        #[arg(
            long,
            default_value_t = false,
            conflicts_with = "hardware_flow_control"
        )]
        software_flow_control: bool,
    },
    #[cfg(feature = "wasm")]
    Wasm { read_fn: String, write_fn: String },
    #[cfg(feature = "wasm")]
    MessageChannel {},
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
            SessionSubcommand::Loopback { initial } => SessionConfig::Loopback(LoopbackConfig {
                initial: initial.unwrap_or_default(),
            }),
            SessionSubcommand::Exec {
                command,
                no_pty,
                #[cfg(feature = "pty")]
                rows,
                #[cfg(feature = "pty")]
                cols,
            } => {
                if !no_pty {
                    #[cfg(feature = "pty")]
                    {
                        return Ok(SessionConfig::ExecPty(ExecPtyConfig {
                            cmd: command.to_string_lossy().to_string(),
                            rows,
                            cols,
                        }));
                    }
                    #[cfg(not(feature = "pty"))]
                    {
                        return Err(Cow::Borrowed(
                            "PTY mode not available (pty feature not enabled)",
                        ));
                    }
                } else {
                    SessionConfig::Exec(ExecConfig {
                        command: command.to_string_lossy().to_string(),
                    })
                }
            }
            SessionSubcommand::Pipe {
                read_write,
                read,
                write,
            } => {
                if let Some(read_write) = read_write {
                    SessionConfig::Pipe(PipeConfig { path: read_write })
                } else if let Some(read) = read {
                    SessionConfig::Pipes(PipesConfig {
                        rx: read,
                        tx: write.unwrap(),
                    })
                } else {
                    unreachable!()
                }
            }
            #[cfg(feature = "serial")]
            SessionSubcommand::Serial {
                path,
                baud_rate,
                data_bits,
                stop_bits,
                hardware_flow_control,
                software_flow_control,
            } => {
                let flow_control = if hardware_flow_control {
                    Some(SerialFlowControl::Hardware)
                } else if software_flow_control {
                    Some(SerialFlowControl::Software)
                } else {
                    None
                };
                SessionConfig::Serial(SerialConfig {
                    path,
                    baud_rate,
                    data_bits,
                    stop_bits,
                    flow_control,
                })
            }
            #[cfg(feature = "wasm")]
            SessionSubcommand::Wasm { read_fn, write_fn } => {
                SessionConfig::Wasm(WasmConfig { read_fn, write_fn })
            }
            #[cfg(feature = "wasm")]
            SessionSubcommand::MessageChannel {} => {
                SessionConfig::MessageChannel(MessageChannelConfig {})
            }
        })
    }
}
