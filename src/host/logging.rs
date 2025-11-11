use std::fs::File;
use std::io::{IsTerminal, stdout};

use tracing_subscriber::filter::{LevelFilter, Targets};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

pub fn setup_logging_file(level: tracing::Level) {
    let tempdir = std::env::temp_dir();
    let logfile = tempdir.join("blaze-vt.log");

    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(File::create(logfile).unwrap())
        .log_internal_errors(false);

    tracing_subscriber::registry()
        .with(fmt_layer)
        .with(logging_targets(level))
        .init();
}

pub fn setup_logging_stdio(level: tracing::Level) {
    let format = tracing_subscriber::fmt::format()
        .with_target(false)
        .with_line_number(false)
        .with_level(false)
        .without_time();

    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_ansi(stdout().is_terminal())
        .event_format(format)
        .log_internal_errors(false);

    tracing_subscriber::registry()
        .with(fmt_layer)
        .with(logging_targets(level))
        .init();
}

pub fn setup_logging_debugger(
    level: tracing::Level,
    trace_collector: i8051_debug_tui::TracingCollector,
) {
    tracing_subscriber::registry()
        .with(trace_collector)
        .with(logging_targets(level))
        .init();
}

fn logging_targets(level: tracing::Level) -> Targets {
    Targets::new()
        .with_target("wgpu_core", LevelFilter::OFF)
        .with_target("winit", LevelFilter::OFF)
        .with_target("wgpu_hal", LevelFilter::OFF)
        .with_target("naga", LevelFilter::OFF)
        .with_default(LevelFilter::from_level(level))
}
