use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, Duration, Local, NaiveTime, Weekday};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use std::{
    fs,
    path::PathBuf,
    thread::{self, JoinHandle},
    time::Duration as StdDuration,
};
use tokio::sync::watch;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum ScanMode {
    Quick,
    Full,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub values: serde_json::Value,
}
#[derive(Debug, Clone)]
pub enum Command {
    Scan {
        mode: ScanMode,
    },
    CancelScan,
    MakeBrief {
        posting_id: String,
    },
    MakeBriefs {
        posting_ids: Vec<String>,
    },
    Apply {
        posting_id: String,
    },
    Unapply {
        posting_ids: Vec<String>,
    },
    SaveSettings {
        settings: Settings,
    },
    RefreshConfig,
    /// Re-send the current dashboard snapshot. The event channel keeps no
    /// retained value, so a view that subscribes after the engine's first
    /// emission asks for one with this command.
    Snapshot,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorKind {
    Busy,
    NotImplemented,
    General,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ScanState {
    Idle,
    Running { mode: ScanMode },
    Error(String),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LastScan {
    pub mode: String,
    pub status: String,
    pub started_at: String,
    pub finished_at: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardSnapshot {
    pub live_postings: i64,
    pub seen_count: i64,
    pub last_scan: Option<LastScan>,
    pub database_path: String,
    pub has_database: bool,
    pub config_files: usize,
    /// The engine's scan state, so a snapshot taken after startup reports
    /// `Idle` instead of leaving the panel on its "starting" placeholder.
    pub scan_state: ScanState,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EngineEvent {
    Dashboard(DashboardSnapshot),
    ScanStatus(ScanState),
    BriefUpdated {
        posting_id: String,
        state: String,
    },
    Error {
        kind: ErrorKind,
        message: String,
        source: String,
    },
}

pub struct EngineHandle {
    pub commands: flume::Sender<Command>,
    pub events: flume::Receiver<EngineEvent>,
    cancel: watch::Sender<bool>,
    join: Option<JoinHandle<()>>,
}
impl EngineHandle {
    pub fn shutdown(mut self) -> Result<()> {
        let _ = self.cancel.send(true);
        if let Some(join) = self.join.take() {
            join.join()
                .map_err(|_| anyhow::anyhow!("engine thread panicked"))?;
        }
        Ok(())
    }
}
impl Drop for EngineHandle {
    fn drop(&mut self) {
        let _ = self.cancel.send(true);
    }
}

pub struct Engine;
impl Engine {
    pub fn spawn(db_path: PathBuf) -> Result<EngineHandle> {
        let (commands, command_rx) = flume::unbounded();
        let (event_tx, events) = flume::unbounded();
        let (cancel, cancel_rx) = watch::channel(false);
        let thread_cancel = cancel.clone();
        let command_sender = commands.clone();
        let join = thread::Builder::new()
            .name("jh-engine".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                match runtime {
                    Ok(runtime) => runtime.block_on(run_engine(
                        db_path,
                        command_sender,
                        command_rx,
                        event_tx,
                        cancel_rx,
                    )),
                    Err(error) => {
                        let _ = event_tx.send(EngineEvent::Error {
                            kind: ErrorKind::General,
                            message: error.to_string(),
                            source: "runtime".into(),
                        });
                    }
                }
            })
            .context("could not start engine thread")?;
        Ok(EngineHandle {
            commands,
            events,
            cancel: thread_cancel,
            join: Some(join),
        })
    }
}

async fn run_engine(
    db_path: PathBuf,
    command_sender: flume::Sender<Command>,
    commands: flume::Receiver<Command>,
    events: flume::Sender<EngineEvent>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut pool: Option<SqlitePool> = None;
    let mut scan_state = ScanState::Idle;
    match open_database(&db_path).await {
        Ok(p) => pool = Some(p),
        Err(e) => emit_error(&events, ErrorKind::General, format!("{e:#}"), "database"),
    }
    emit_dashboard(
        &events,
        snapshot(&pool, &db_path, read_config_files(), scan_state.clone()).await,
    );
    let mut scanning = false;
    let mut scan_cancel: Option<watch::Sender<bool>> = None;
    let mut next = next_slot(Local::now());
    let (done_tx, done_rx) = flume::unbounded::<ScanState>();
    loop {
        let wait = next
            .signed_duration_since(Local::now())
            .to_std()
            .unwrap_or(StdDuration::ZERO);
        tokio::select! {
            changed = shutdown.changed() => { if changed.is_err() || *shutdown.borrow() { if let Some(tx) = &scan_cancel { let _ = tx.send(true); } break; } }
            _ = tokio::time::sleep(wait) => {
                next = next_slot(Local::now());
                if !scanning { let _ = command_sender.send(Command::Scan { mode: ScanMode::Quick }); }
            }
            final_state = done_rx.recv_async(), if scanning => {
                scanning = false;
                scan_cancel = None;
                if let Ok(state) = final_state {
                    scan_state = state.clone();
                    let _ = events.send(EngineEvent::ScanStatus(state));
                }
            }
            command = commands.recv_async() => match command {
                Ok(Command::Scan { mode }) => {
                    if scanning { emit_error(&events, ErrorKind::Busy, "a scan is already running".into(), "scan"); continue; }
                    match acquire_scan_lock() {
                        Ok(lock) => {
                            scanning = true;
                            scan_state = ScanState::Running { mode };
                            let (cancel_tx, cancel_rx) = watch::channel(false);
                            scan_cancel = Some(cancel_tx);
                            let p = pool.clone(); let tx = events.clone(); let done = done_tx.clone();
                            let path = db_path.clone();
                            let cfg_count = read_config_files();
                            let _ = events.send(EngineEvent::ScanStatus(scan_state.clone()));
                            tokio::spawn(async move { run_scan(ScanJob { pool: p, db_path: path, mode, lock, events: tx, done, cancel: cancel_rx, config_files: cfg_count }).await; });
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => emit_error(&events, ErrorKind::Busy, "another process holds the scan lock".into(), "scan"),
                        Err(e) => emit_error(&events, ErrorKind::General, e.to_string(), "scan"),
                    }
                }
                Ok(Command::CancelScan) => { if let Some(tx) = &scan_cancel { let _ = tx.send(true); } }
                Ok(Command::RefreshConfig) => {
                    let count = read_config_files();
                    emit_dashboard(&events, snapshot(&pool, &db_path, count, scan_state.clone()).await);
                }
                Ok(Command::Snapshot) => {
                    let count = read_config_files();
                    emit_dashboard(&events, snapshot(&pool, &db_path, count, scan_state.clone()).await);
                }
                Ok(other) => emit_error(&events, ErrorKind::NotImplemented, "not implemented yet".into(), command_name(&other)),
                Err(_) => break,
            },
        }
    }
    if let Some(pool) = pool {
        pool.close().await;
    }
}

struct ScanJob {
    pool: Option<SqlitePool>,
    db_path: PathBuf,
    mode: ScanMode,
    lock: std::fs::File,
    events: flume::Sender<EngineEvent>,
    done: flume::Sender<ScanState>,
    cancel: watch::Receiver<bool>,
    config_files: usize,
}
async fn run_scan(job: ScanJob) {
    let ScanJob {
        pool,
        db_path,
        mode,
        lock,
        events,
        done,
        mut cancel,
        config_files,
    } = job;
    let started = Local::now().to_rfc3339();
    let result = async {
        let pool = match pool {
            Some(pool) => pool,
            None => open_database(&db_path).await?,
        };
        let insert = sqlx::query("INSERT INTO scan_runs(mode,status,started_at,matched,stats_json) VALUES(?, 'running', ?, 0, '{\"stub\":true}')")
            .bind(match mode { ScanMode::Quick => "quick", ScanMode::Full => "full" }).bind(&started).execute(&pool).await?;
        let run_id = insert.last_insert_rowid();
        let pause = tokio::time::sleep(StdDuration::from_millis(900)); tokio::pin!(pause);
        let cancelled = tokio::select! { _ = &mut pause => false, changed = cancel.changed() => changed.is_ok() && *cancel.borrow() };
        let status = if cancelled { "cancelled" } else { "ok" };
        sqlx::query("UPDATE scan_runs SET status = ?, finished_at = ?, matched = 0, stats_json = '{\"stub\":true}' WHERE id = ?")
            .bind(status).bind(Local::now().to_rfc3339()).bind(run_id).execute(&pool).await?;
        emit_dashboard(&events, snapshot(&Some(pool), &db_path, config_files, ScanState::Idle).await);
        Ok::<bool, anyhow::Error>(cancelled)
    }.await;
    match result {
        Ok(cancelled) => {
            if cancelled {
                let _ = events.send(EngineEvent::Error {
                    kind: ErrorKind::General,
                    message: "scan cancelled".into(),
                    source: "scan".into(),
                });
            }
            let _ = done.send(ScanState::Idle);
        }
        Err(error) => {
            emit_error(&events, ErrorKind::General, error.to_string(), "scan");
            let _ = done.send(ScanState::Error(error.to_string()));
        }
    }
    drop(lock);
}

/// Opens the engine's database, creating the file and any missing parent
/// directory, then runs the migrations. A missing database is not an error:
/// the engine creates it the first time it starts.
async fn open_database(path: &std::path::Path) -> Result<SqlitePool> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .with_context(|| format!("could not create {}", parent.display()))?;
    }
    let path = path
        .to_str()
        .with_context(|| format!("database path is not UTF-8: {}", path.display()))?;
    let pool = jh_db::connect(path).await?;
    jh_db::migrate(&pool).await?;
    Ok(pool)
}
async fn snapshot(
    pool: &Option<SqlitePool>,
    path: &std::path::Path,
    config_files: usize,
    scan_state: ScanState,
) -> DashboardSnapshot {
    let mut out = DashboardSnapshot {
        live_postings: 0,
        seen_count: 0,
        last_scan: None,
        database_path: path.display().to_string(),
        has_database: pool.is_some(),
        config_files,
        scan_state,
    };
    if let Some(pool) = pool {
        out.live_postings =
            sqlx::query("SELECT COUNT(*) AS n FROM postings WHERE retired_at IS NULL")
                .fetch_one(pool)
                .await
                .ok()
                .and_then(|r| r.try_get("n").ok())
                .unwrap_or(0);
        out.seen_count = sqlx::query("SELECT COUNT(*) AS n FROM seen")
            .fetch_one(pool)
            .await
            .ok()
            .and_then(|r| r.try_get("n").ok())
            .unwrap_or(0);
        out.last_scan = sqlx::query(
            "SELECT mode,status,started_at,finished_at FROM scan_runs ORDER BY id DESC LIMIT 1",
        )
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .map(|r| LastScan {
            mode: r.get("mode"),
            status: r.get("status"),
            started_at: r.get("started_at"),
            finished_at: r.get("finished_at"),
        });
    }
    out
}
fn emit_dashboard(tx: &flume::Sender<EngineEvent>, snapshot: DashboardSnapshot) {
    let _ = tx.send(EngineEvent::Dashboard(snapshot));
}
fn emit_error(tx: &flume::Sender<EngineEvent>, kind: ErrorKind, message: String, source: &str) {
    let _ = tx.send(EngineEvent::Error {
        kind,
        message,
        source: source.into(),
    });
}
fn command_name(command: &Command) -> &'static str {
    match command {
        Command::MakeBrief { .. } => "brief",
        Command::MakeBriefs { .. } => "briefs",
        Command::Apply { .. } => "apply",
        Command::Unapply { .. } => "unapply",
        Command::SaveSettings { .. } => "settings",
        _ => "command",
    }
}
fn read_config_files() -> usize {
    fs::read_dir("data/config")
        .map(|it| {
            it.flatten()
                .filter(|e| {
                    e.path()
                        .extension()
                        .is_some_and(|x| x == "yaml" || x == "yml")
                })
                .filter_map(|e| fs::read(e.path()).ok())
                .count()
        })
        .unwrap_or(0)
}
fn acquire_scan_lock() -> std::io::Result<std::fs::File> {
    use std::os::fd::AsRawFd;
    let path = PathBuf::from("data/state/.scan.lock");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    // SAFETY: flock uses the valid file descriptor and does not retain pointers.
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == -1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(file)
}

pub fn next_slot(now: DateTime<Local>) -> DateTime<Local> {
    let slots = [
        NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        NaiveTime::from_hms_opt(13, 0, 0).unwrap(),
        NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
    ];
    for day_offset in 0..8 {
        let date = now.date_naive() + Duration::days(day_offset);
        let weekday = date.weekday();
        if matches!(weekday, Weekday::Sat | Weekday::Sun) {
            continue;
        }
        for slot in slots {
            let local = date.and_time(slot).and_local_timezone(Local).single();
            if let Some(candidate) = local.filter(|candidate| *candidate > now) {
                return candidate;
            }
        }
    }
    let fallback = now.date_naive() + Duration::days(7);
    fallback
        .and_hms_opt(9, 0, 0)
        .unwrap()
        .and_local_timezone(Local)
        .single()
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;
    static SCAN_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn temp_path(prefix: &str) -> PathBuf {
        std::env::temp_dir().join(format!("jh-engine-{prefix}-{}.sqlite", std::process::id()))
    }
    fn remove_database(path: &std::path::Path) {
        let _ = fs::remove_file(path);
        for suffix in ["-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }
    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }
    /// The next dashboard event, skipping the others. Panics on timeout.
    fn next_dashboard(rx: &flume::Receiver<EngineEvent>) -> DashboardSnapshot {
        loop {
            match rx.recv_timeout(StdDuration::from_secs(5)).unwrap() {
                EngineEvent::Dashboard(snapshot) => return snapshot,
                _ => continue,
            }
        }
    }
    fn scan_runs(path: &std::path::Path) -> Vec<(String, String)> {
        block_on(async {
            let pool = open_database(path).await.unwrap();
            let rows = sqlx::query("SELECT mode, status FROM scan_runs ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap()
                .into_iter()
                .map(|r| (r.get("mode"), r.get("status")))
                .collect();
            pool.close().await;
            rows
        })
    }

    fn at(day: i64, h: u32, m: u32) -> DateTime<Local> {
        let date = chrono::NaiveDate::from_ymd_opt(2026, 11, day as u32).unwrap();
        date.and_hms_opt(h, m, 0)
            .unwrap()
            .and_local_timezone(Local)
            .single()
            .unwrap()
    }
    #[test]
    fn next_slot_before_nine() {
        assert_eq!(next_slot(at(16, 8, 0)).hour(), 9);
    }
    #[test]
    fn next_slot_between_slots() {
        assert_eq!(next_slot(at(16, 10, 0)).hour(), 13);
    }
    #[test]
    fn next_slot_after_seventeen() {
        assert_eq!(next_slot(at(16, 18, 0)).day(), 17);
    }
    #[test]
    fn next_slot_friday_evening() {
        assert_eq!(next_slot(at(13, 18, 0)).day(), 16);
    }
    #[test]
    fn next_slot_saturday() {
        assert_eq!(next_slot(at(14, 10, 0)).weekday(), Weekday::Mon);
    }
    #[test]
    fn command_round_trip() {
        let _serial = SCAN_TEST_LOCK.lock().unwrap();
        let path = temp_path("round-trip");
        remove_database(&path);
        let handle = Engine::spawn(path.clone()).unwrap();
        let rx = handle.events.clone();
        let _ = rx.recv_timeout(StdDuration::from_secs(5)).unwrap();
        handle
            .commands
            .send(Command::Scan {
                mode: ScanMode::Quick,
            })
            .unwrap();
        assert!(matches!(
            rx.recv_timeout(StdDuration::from_secs(5)).unwrap(),
            EngineEvent::ScanStatus(ScanState::Running { .. })
        ));
        assert!(matches!(
            rx.recv_timeout(StdDuration::from_secs(5)).unwrap(),
            EngineEvent::Dashboard(_)
        ));
        assert!(matches!(
            rx.recv_timeout(StdDuration::from_secs(5)).unwrap(),
            EngineEvent::ScanStatus(ScanState::Idle)
        ));
        let rows = scan_runs(&path);
        assert_eq!(rows.len(), 1);
        handle.shutdown().unwrap();
        remove_database(&path);
    }
    #[test]
    fn busy_lock() {
        let _serial = SCAN_TEST_LOCK.lock().unwrap();
        let file = acquire_scan_lock().unwrap();
        let path = temp_path("busy");
        remove_database(&path);
        let handle = Engine::spawn(path.clone()).unwrap();
        let rx = handle.events.clone();
        assert!(matches!(
            rx.recv_timeout(StdDuration::from_secs(5)).unwrap(),
            EngineEvent::Dashboard(_)
        ));
        handle
            .commands
            .send(Command::Scan {
                mode: ScanMode::Quick,
            })
            .unwrap();
        assert!(matches!(
            rx.recv_timeout(StdDuration::from_secs(5)).unwrap(),
            EngineEvent::Error {
                kind: ErrorKind::Busy,
                ..
            }
        ));
        handle.shutdown().unwrap();
        drop(file);
        remove_database(&path);
    }
    /// A first start with no `data/state` directory must create the file, run
    /// the migrations, and answer a `Snapshot` with a fresh `Dashboard`.
    #[test]
    fn engine_creates_missing_database() {
        let _serial = SCAN_TEST_LOCK.lock().unwrap();
        let root =
            std::env::temp_dir().join(format!("jh-engine-missing-parent-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let path = root.join("data").join("state").join("jobhunt.db");
        assert!(!path.exists());
        let handle = Engine::spawn(path.clone()).unwrap();
        let rx = handle.events.clone();
        // The engine's own startup snapshot; the file is open by then.
        let startup = next_dashboard(&rx);
        assert!(startup.has_database, "startup snapshot: {startup:?}");
        assert!(path.exists(), "engine did not create {}", path.display());
        // Ask for a fresh snapshot the way the view does on open.
        handle.commands.send(Command::Snapshot).unwrap();
        let snapshot = next_dashboard(&rx);
        assert!(snapshot.has_database, "snapshot: {snapshot:?}");
        assert_eq!(snapshot.database_path, path.display().to_string());
        let migrations: i64 = block_on(async {
            let pool = open_database(&path).await.unwrap();
            let applied = sqlx::query("SELECT COUNT(*) FROM _sqlx_migrations")
                .fetch_one(&pool)
                .await
                .unwrap()
                .get(0);
            pool.close().await;
            applied
        });
        assert!(migrations > 0, "migrations did not run");
        handle.shutdown().unwrap();
        let _ = fs::remove_dir_all(&root);
    }
    /// Opening the window asks for a snapshot, and the answer must describe a
    /// running engine: a database that exists and an idle scan state.
    #[test]
    fn first_dashboard_reports_idle() {
        let _serial = SCAN_TEST_LOCK.lock().unwrap();
        let path = temp_path("first-dashboard");
        remove_database(&path);
        let handle = Engine::spawn(path.clone()).unwrap();
        let rx = handle.events.clone();
        handle.commands.send(Command::Snapshot).unwrap();
        let first = match rx.recv_timeout(StdDuration::from_secs(5)).unwrap() {
            EngineEvent::Dashboard(snapshot) => snapshot,
            other => panic!("first event was {other:?}, not a dashboard"),
        };
        assert!(
            matches!(first.scan_state, ScanState::Idle),
            "state was {:?}",
            first.scan_state
        );
        assert!(first.has_database, "snapshot: {first:?}");
        // The engine's own startup snapshot and the answer to `Snapshot` are
        // both dashboards; the second proves the command path works.
        let second = next_dashboard(&rx);
        assert!(matches!(second.scan_state, ScanState::Idle));
        handle.shutdown().unwrap();
        remove_database(&path);
    }
    /// `Scan now` on a fresh database writes one row, marks it `ok`, and puts
    /// the engine back to idle.
    #[test]
    fn scan_writes_one_run_and_returns_to_idle() {
        let _serial = SCAN_TEST_LOCK.lock().unwrap();
        let path = temp_path("scan-run");
        remove_database(&path);
        let handle = Engine::spawn(path.clone()).unwrap();
        let rx = handle.events.clone();
        let _ = next_dashboard(&rx);
        handle
            .commands
            .send(Command::Scan {
                mode: ScanMode::Quick,
            })
            .unwrap();
        let mut idle = false;
        let mut snapshots = Vec::new();
        while let Ok(event) = rx.recv_timeout(StdDuration::from_secs(5)) {
            match event {
                EngineEvent::ScanStatus(ScanState::Idle) => {
                    idle = true;
                    break;
                }
                EngineEvent::Dashboard(snapshot) => snapshots.push(snapshot),
                _ => {}
            }
        }
        assert!(idle, "the engine never returned to idle");
        let after_scan = snapshots.last().expect("a dashboard after the scan");
        assert!(matches!(after_scan.scan_state, ScanState::Idle));
        assert_eq!(
            after_scan.last_scan.as_ref().map(|s| s.status.as_str()),
            Some("ok")
        );
        let rows = scan_runs(&path);
        assert_eq!(rows, vec![("quick".to_string(), "ok".to_string())]);
        handle.shutdown().unwrap();
        remove_database(&path);
    }
}
