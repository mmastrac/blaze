use crate::session::io_session::boot_io;

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionConfig {
    /// Loopback mode (no external connection)
    Loopback(loopback::LoopbackConfig),
    /// Single bidirectional pipe
    Pipe(pipe::PipeConfig),
    /// Separate read and write pipes
    Pipes(pipe::PipesConfig),
    /// Execute a command and connect to its pty
    Exec(exec::ExecConfig),
    /// Execute a command and connect to its pty
    #[cfg(feature = "pty")]
    ExecPty(pty::ExecPtyConfig),
    /// Connect to a serial port
    #[cfg(feature = "serial")]
    Serial(serial::SerialConfig),
    /// Connect to an external JS source on `globalThis`.
    #[cfg(feature = "wasm")]
    Wasm(wasm::WasmConfig),
    /// Connect to the window's message handler.
    #[cfg(feature = "wasm")]
    MessageChannel(wasm::MessageChannelConfig),
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self::Loopback(loopback::LoopbackConfig {
            initial: String::new(),
        })
    }
}

impl SessionConfig {
    pub fn start(self) -> std::io::Result<SessionParts> {
        use crate::session::Session;
        match self {
            SessionConfig::Loopback(config) => config.boot(),
            SessionConfig::Pipe(config) => boot_io(config),
            SessionConfig::Pipes(config) => boot_io(config),
            SessionConfig::Exec(config) => boot_io(config),
            #[cfg(feature = "pty")]
            SessionConfig::ExecPty(config) => boot_io(config),
            #[cfg(feature = "serial")]
            SessionConfig::Serial(config) => boot_io(config),
            _ => Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!(
                    "Cannot boot session {:?} with multi-threaded endpoint",
                    self
                ),
            )),
        }
    }

    pub fn start_unsend(self) -> std::io::Result<SessionPartsUnsend> {
        match self {
            #[cfg(feature = "wasm")]
            SessionConfig::Wasm(config) => {
                use crate::session::SessionUnsend;
                config.boot()
            }
            #[cfg(feature = "wasm")]
            SessionConfig::MessageChannel(config) => {
                use crate::session::SessionUnsend;
                config.boot()
            }
            _ => self.start().map(|parts| parts.into()),
        }
    }
}
