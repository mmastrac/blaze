pub mod headless;

#[cfg(feature = "tui")]
pub mod ratatui;

#[cfg(feature = "tui")]
pub mod text;

#[cfg(feature = "tui")]
pub mod text_terminal;

#[cfg(feature = "graphics")]
pub mod framebuffer;
