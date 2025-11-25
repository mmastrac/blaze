use ssu::server::Server;

use clap::{Arg, Parser};

#[derive(Parser)]
struct Args {
    #[clap(short, long)]
    sessions: u8,
}

pub fn main() {
    let args = Args::parse();
    let server = Server::new(args.sessions);
}
