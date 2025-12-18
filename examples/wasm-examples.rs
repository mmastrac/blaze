use std::{fs, io, path::PathBuf};

use axum::routing::get_service;
use clap::Parser;

#[derive(Debug, clap::Parser)]
enum Subcommand {
    Serve {
        #[arg(long, default_value = "8000")]
        port: u16,
    },
    Build {
        #[arg(long)]
        out_dir: PathBuf,
    },
}

fn main() -> io::Result<()> {
    // SAFETY: Setting this before anything else runs
    unsafe { std::env::set_var("RUSTFLAGS", "-Awarnings") };

    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is not set"));
    let docs_dir = manifest_dir.join("docs");
    let examples_dir = manifest_dir.join("examples");
    let wasm_target_dir = manifest_dir.join("target/wasm-examples/blaze-vt");

    let args = Subcommand::parse();
    let mut tempdir_maybe = None;
    let out_dir = match &args {
        Subcommand::Serve { port } => {
            let tempdir = tempfile::tempdir()?;
            let out_dir = tempdir.path().join(format!("serve-{}", port));
            tempdir_maybe = Some(tempdir);
            out_dir
        }
        Subcommand::Build { out_dir } => out_dir.clone(),
    };
    fs::create_dir_all(&out_dir)?;

    // Build the WASM binary
    eprintln!("Building WASM binary...");
    cargo_run_wasm::RunWasm::new()
        .with_build_only(true)
        .with_css(
            r#"
html, body {
    margin: 0;
    padding: 0;
    width: 100vw;
    height: 100vh;
    overflow: hidden;
}
        "#,
        )
        .with_bin(Some("blaze-vt".into()))
        .with_cargo_build_args(vec![
            "--no-default-features".into(),
            "--features=wasm".into(),
            "-q".into(),
        ])
        .with_profile(Some("release".into()))
        .run()
        .map_err(io::Error::other)?;

    eprintln!("Copying docs to {:?}...", out_dir);
    fs::copy(docs_dir.join("index.html"), out_dir.join("index.html"))?;
    let wasm_out_dir = out_dir.join("wasm");
    fs::create_dir_all(&wasm_out_dir)?;
    for entry in fs::read_dir(wasm_target_dir)?.flatten() {
        if !entry.metadata()?.is_file() {
            continue;
        }
        fs::copy(entry.path(), wasm_out_dir.join(entry.file_name()))?;
    }
    let examples_out_dir = out_dir.join("examples");
    fs::create_dir_all(&examples_out_dir)?;
    let wasm_examples = examples_dir.join("wasm");
    let wasm_html = wasm_examples.join("index.html");
    for entry in fs::read_dir(examples_dir.join("wasm"))?.flatten() {
        if !entry.metadata()?.is_dir() {
            continue;
        }
        let example_out_dir = examples_out_dir.join(entry.file_name());
        fs::create_dir_all(&example_out_dir)?;
        fs::copy(&wasm_html, example_out_dir.join("index.html"))?;
        let entry_path = entry.path();
        fs::copy(
            entry_path.join("index.js"),
            example_out_dir.join("index.js"),
        )?;
        let example_build_dir = entry_path.join("build");
        if example_build_dir.exists() {
            for entry in fs::read_dir(example_build_dir)?.flatten() {
                if !entry.metadata()?.is_file() {
                    continue;
                }
                fs::copy(entry.path(), example_out_dir.join(entry.file_name()))?;
            }
        }
    }

    if let Subcommand::Serve { port } = args {
        let serve_dir =
            tower_http::services::ServeDir::new(&out_dir).append_index_html_on_directories(true);

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async move {
            tokio::spawn(async move {
                _ = tokio::signal::ctrl_c().await;
                eprintln!();
                eprintln!("Ctrl-C received, shutting down...");
                if let Some(tempdir) = tempdir_maybe.take() {
                    eprintln!("Removing temporary directory {:?}", tempdir.path());
                }
                std::process::exit(0);
            });
            let tcp = tokio::net::TcpListener::bind(&format!("0.0.0.0:{}", port)).await?;
            eprintln!("Listening on http://{}...", tcp.local_addr().unwrap());
            axum::serve(tcp, get_service(serve_dir)).await?;
            Ok::<_, io::Error>(())
        })?;
    }

    Ok(())
}
