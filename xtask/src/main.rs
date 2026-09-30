use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::thread;
use std::time::{Duration, Instant};

type Error = Box<dyn std::error::Error + Send + Sync>;

const SCRIPT_TIMEOUT: Duration = Duration::from_secs(600);

struct Machine {
    name: &'static str,
    rom: &'static str,
    args: &'static [&'static str],
}

const MACHINES: &[Machine] = &[Machine {
    name: "vt420",
    rom: "roms/vt420/23-068E9-00.bin",
    args: &[],
}];

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("screenshots") => screenshots(&args[1..]),
        _ => Err("usage: cargo xtask screenshots [script...]".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn screenshots(filter: &[String]) -> Result<(), Error> {
    let root = root();
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = Command::new(cargo)
        .current_dir(&root)
        .args(["build", "--release", "--bin", "blaze-vt"])
        .status()?;
    if !status.success() {
        return Err("building blaze-vt failed".into());
    }
    let target = env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"));
    let binary = target.join("release").join("blaze-vt");
    let logs = target.join("xtask");
    fs::create_dir_all(&logs)?;

    let mut jobs = vec![];
    for machine in MACHINES {
        let dir = root.join("docs/scripts").join(machine.name);
        let mut scripts: Vec<PathBuf> = fs::read_dir(&dir)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<_, _>>()?;
        scripts.retain(|path| path.extension().is_some_and(|ext| ext == "script"));
        scripts.sort();
        for script in scripts {
            let name = script.strip_prefix(&root)?.display().to_string();
            if filter.is_empty() || filter.iter().any(|f| name.contains(f.as_str())) {
                jobs.push((machine, script, name));
            }
        }
    }
    if jobs.is_empty() {
        return Err("no scripts matched".into());
    }

    let handles: Vec<_> = jobs
        .into_iter()
        .map(|(machine, script, name)| {
            let log = logs.join(format!(
                "{}-{}.log",
                machine.name,
                script.file_stem().unwrap().display()
            ));
            let (root, binary) = (root.clone(), binary.clone());
            thread::spawn(move || (run_script(&root, &binary, machine, &script, &log), name))
        })
        .collect();
    let mut failed = 0;
    for handle in handles {
        let (result, name) = handle.join().unwrap();
        match result {
            Ok(elapsed) => eprintln!("ok      {name} ({:.1}s)", elapsed.as_secs_f64()),
            Err(e) => {
                eprintln!("FAILED  {name}: {e}");
                failed += 1;
            }
        }
    }
    if failed > 0 {
        return Err(format!("{failed} script(s) failed").into());
    }
    Ok(())
}

fn run_script(
    root: &Path,
    binary: &Path,
    machine: &Machine,
    script: &Path,
    log: &Path,
) -> Result<Duration, Error> {
    let log_file = fs::File::create(log)?;
    let start = Instant::now();
    let mut child = Command::new(binary)
        .current_dir(root)
        .args(["--machine", machine.name, "--rom", machine.rom])
        .args(machine.args)
        .arg("--script")
        .arg(script)
        .stdout(log_file.try_clone()?)
        .stderr(log_file)
        .spawn()?;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() > SCRIPT_TIMEOUT {
            child.kill()?;
            return Err(format!("timed out, see {}", log.display()).into());
        }
        thread::sleep(Duration::from_millis(100));
    };
    if !status.success() {
        return Err(format!("{status}, see {}", log.display()).into());
    }
    fs::remove_file(log)?;
    Ok(start.elapsed())
}
