//! The Unix-socket transport between the menu-bar server and the desktop app.
//!
//! One JSON object per line, in both directions. The server accepts any number
//! of clients and broadcasts every engine event to all of them, so a second app
//! window (or a test client) sees the same state.

use anyhow::{Context, Result};
use flume::{Receiver, Sender};
use jh_protocol::{Command, EngineEvent, WireMessage};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread,
};

type Client = Arc<Mutex<UnixStream>>;
type ClientList = Arc<Mutex<Vec<(u64, Client)>>>;

/// True when a server already answers on this socket. A stale socket file from
/// a crashed process refuses the connection, so the caller may replace it.
pub fn socket_is_live(path: &Path) -> bool {
    UnixStream::connect(path).is_ok()
}

pub fn serve(
    path: PathBuf,
    commands: Sender<Command>,
    events: Receiver<EngineEvent>,
) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .with_context(|| format!("could not create {}", parent.display()))?;
    }
    let _ = fs::remove_file(&path);
    let listener =
        UnixListener::bind(&path).with_context(|| format!("could not bind {}", path.display()))?;
    let clients: ClientList = Arc::new(Mutex::new(Vec::new()));

    // One writer drains the engine's event stream and fans it out. A client
    // that has gone away is dropped from the list on its first failed write.
    let broadcast_clients = clients.clone();
    thread::Builder::new()
        .name("jh-ipc-broadcast".into())
        .spawn(move || {
            while let Ok(event) = events.recv() {
                let Ok(payload) = serde_json::to_string(&WireMessage::Event { event }) else {
                    continue;
                };
                let mut list = broadcast_clients.lock().expect("client list poisoned");
                list.retain(|(_, client)| {
                    let mut stream = client.lock().expect("client stream poisoned");
                    stream
                        .write_all(payload.as_bytes())
                        .and_then(|_| stream.write_all(b"\n"))
                        .and_then(|_| stream.flush())
                        .is_ok()
                });
            }
        })
        .context("could not start the event broadcaster")?;

    let next_id = Arc::new(AtomicU64::new(1));
    thread::Builder::new()
        .name("jh-ipc-accept".into())
        .spawn(move || {
            for incoming in listener.incoming() {
                let Ok(stream) = incoming else {
                    continue;
                };
                let id = next_id.fetch_add(1, Ordering::Relaxed);
                let Ok(writer) = stream.try_clone() else {
                    continue;
                };
                let writer: Client = Arc::new(Mutex::new(writer));
                clients
                    .lock()
                    .expect("client list poisoned")
                    .push((id, writer));
                let commands = commands.clone();
                let readers = clients.clone();
                let _ = thread::Builder::new()
                    .name(format!("jh-ipc-client-{id}"))
                    .spawn(move || {
                        let reader = BufReader::new(stream);
                        for line in reader.lines() {
                            let Ok(line) = line else {
                                break;
                            };
                            if line.trim().is_empty() {
                                continue;
                            }
                            match serde_json::from_str::<WireMessage>(&line) {
                                Ok(WireMessage::Command { command }) => {
                                    let _ = commands.send(command);
                                }
                                Ok(WireMessage::Event { .. }) => {
                                    eprintln!("[server] client sent an event; ignored");
                                }
                                Err(error) => {
                                    eprintln!("[server] bad command line: {error}");
                                }
                            }
                        }
                        readers
                            .lock()
                            .expect("client list poisoned")
                            .retain(|(client_id, _)| *client_id != id);
                    });
            }
        })
        .context("could not start the socket acceptor")?;

    Ok(())
}
