//! The job-hunt server: a menu-bar agent that owns the engine, the SQLite
//! database and the Unix socket the desktop app talks to.
//!
//! Run with no arguments for the normal menu-bar process. `--headless` keeps
//! the same engine and socket without AppKit, which is what tests and a remote
//! shell use.

mod ipc;
#[cfg(target_os = "macos")]
mod status_item;

use anyhow::Result;
use clap::Parser;
use jh_engine::{Command, Engine, EngineHandle, ScanMode};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Parser)]
#[command(name = "jh-server", about = "Job Hunt menu-bar server")]
struct Args {
    /// Database file. Defaults to data/state/jobhunt.db.
    #[arg(long)]
    db: Option<PathBuf>,
    /// Unix socket path. Defaults to data/state/jh.sock.
    #[arg(long)]
    socket: Option<PathBuf>,
    /// Do not install the menu-bar status item; run until killed.
    #[arg(long)]
    headless: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let db_path = args
        .db
        .unwrap_or_else(|| PathBuf::from("data/state/jobhunt.db"));
    let socket_path = args.socket.unwrap_or_else(jh_protocol::default_socket_path);

    // A second copy must not steal the socket from a running server.
    if ipc::socket_is_live(&socket_path) {
        eprintln!(
            "[server] another instance already answers {}",
            socket_path.display()
        );
        return Ok(());
    }

    let engine = Engine::spawn(db_path)?;
    let commands = engine.commands.clone();
    let events = engine.events.clone();
    let handle = Arc::new(Mutex::new(Some(engine)));

    ipc::serve(socket_path.clone(), commands.clone(), events)?;

    if args.headless {
        eprintln!("[server] headless; socket at {}", socket_path.display());
        loop {
            std::thread::sleep(Duration::from_secs(3600));
        }
    }

    #[cfg(target_os = "macos")]
    {
        run_menu_bar(commands, handle, socket_path)?;
    }
    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("[server] the menu bar needs macOS; use --headless");
        let _ = handle;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn run_menu_bar(
    commands: flume::Sender<Command>,
    engine: Arc<Mutex<Option<EngineHandle>>>,
    socket_path: PathBuf,
) -> Result<()> {
    use status_item::{MenuAction, install};

    let (menu_tx, menu_rx) = flume::unbounded();
    install(menu_tx);

    std::thread::Builder::new()
        .name("jh-menu".into())
        .spawn(move || {
            while let Ok(action) = menu_rx.recv() {
                match action {
                    MenuAction::Open => launch_app(&[]),
                    MenuAction::Settings => launch_app(&["--settings"]),
                    MenuAction::ScanNow => {
                        let _ = commands.send(Command::Scan {
                            mode: ScanMode::Quick,
                        });
                    }
                    MenuAction::Quit => {
                        if let Some(engine) = engine.lock().ok().and_then(|mut e| e.take()) {
                            let _ = engine.shutdown();
                        }
                        let _ = std::fs::remove_file(&socket_path);
                        std::process::exit(0);
                    }
                }
            }
        })?;

    status_item::run_app()?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn launch_app(args: &[&str]) {
    let sibling = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join("jh-app")));
    if let Some(path) = sibling.filter(|path| path.exists()) {
        let _ = std::process::Command::new(path).args(args).spawn();
        return;
    }
    let _ = std::process::Command::new("open")
        .arg("-a")
        .arg("Job Hunt")
        .args(args)
        .spawn();
}
