use std::fs::File;
use std::io;

use raw_tty::IntoRawMode;
use ssu::server::run_async;
use ssu::session::{SessionConfig, SessionEndpoint};

use clap::Parser;
use tracing::{info, trace};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
struct Args {
    #[clap(short, long)]
    session: Vec<SessionConfig>,
}

#[tokio::main]
pub async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(File::create("/tmp/ssu.log").unwrap())
        .init();

    info!("Starting SSU server");

    let args = Args::parse();

    trace!("Entering stdin");
    let stdin = io::stdin().into_raw_mode().unwrap();
    trace!("Entering sessions");

    let sessions: Vec<Box<dyn SessionEndpoint + Send + 'static>> = args
        .session
        .into_iter()
        .map(|config| config.start())
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    run_async(sessions).await;

    drop(stdin);
}
