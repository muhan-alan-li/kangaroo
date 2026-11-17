//! The desktop app's side of the Unix-socket contract. It never touches the
//! database: every command goes to the menu-bar server, every event comes back
//! on the same connection.

use anyhow::{Context, Result};
use flume::{Receiver, Sender};
use kangaroo_protocol::{Command, EngineEvent, WireMessage};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::Path,
    thread,
    time::Duration,
};

pub struct Client {
    pub commands: Sender<Command>,
    pub events: Receiver<EngineEvent>,
}

impl Client {
    pub fn connect(path: &Path) -> Result<Self> {
        let stream = UnixStream::connect(path)
            .with_context(|| format!("the server is not listening on {}", path.display()))?;
        let mut writer = stream.try_clone().context("could not clone the socket")?;
        let reader = BufReader::new(stream);
        let (command_tx, command_rx) = flume::unbounded();
        let (event_tx, event_rx) = flume::unbounded();

        thread::Builder::new()
            .name("kangaroo-client-writer".into())
            .spawn(move || {
                for command in command_rx {
                    let Ok(line) = serde_json::to_string(&WireMessage::Command { command }) else {
                        continue;
                    };
                    let sent = writer
                        .write_all(line.as_bytes())
                        .and_then(|_| writer.write_all(b"\n"))
                        .and_then(|_| writer.flush());
                    if sent.is_err() {
                        break;
                    }
                }
            })
            .context("could not start the client writer")?;

        thread::Builder::new()
            .name("kangaroo-client-reader".into())
            .spawn(move || {
                for line in reader.lines() {
                    let Ok(line) = line else {
                        break;
                    };
                    if line.trim().is_empty() {
                        continue;
                    }
                    if let Ok(WireMessage::Event { event }) = serde_json::from_str(&line) {
                        let _ = event_tx.send(event);
                    }
                }
            })
            .context("could not start the client reader")?;

        Ok(Self {
            commands: command_tx,
            events: event_rx,
        })
    }

    /// Connect to the server, starting a sibling `kangaroo-server` if none answers.
    pub fn connect_or_start() -> Result<Self> {
        let path = kangaroo_protocol::default_socket_path();
        if let Ok(client) = Self::connect(&path) {
            return Ok(client);
        }
        if let Some(server) = std::env::current_exe().ok().and_then(|exe| {
            exe.parent()
                .map(|parent| parent.join("kangaroo-server"))
                .filter(|path| path.exists())
        }) {
            let _ = std::process::Command::new(server).spawn();
        }
        for _ in 0..30 {
            thread::sleep(Duration::from_millis(100));
            if let Ok(client) = Self::connect(&path) {
                return Ok(client);
            }
        }
        Self::connect(&path)
    }
}
