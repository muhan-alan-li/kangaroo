use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, Duration, Local, NaiveTime, Weekday};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool, sqlite::SqlitePoolOptions};
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
    Scan { mode: ScanMode },
    CancelScan,
    MakeBrief { posting_id: String },
    MakeBriefs { posting_ids: Vec<String> },
    Apply { posting_id: String },
    Unapply { posting_ids: Vec<String> },
    SaveSettings { settings: Settings },
    RefreshConfig,
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
    if db_path.exists() {
        match connect(&db_path).await {
            Ok(p) => {
                if let Err(e) = migrate(&p).await {
                    emit_error(&events, ErrorKind::General, e.to_string(), "database");
                } else {
                    pool = Some(p);
                }
            }
            Err(e) => emit_error(&events, ErrorKind::General, e.to_string(), "database"),
        }
    }
    emit_dashboard(
        &events,
        snapshot(&pool, &db_path, read_config_files()).await,
    );
    let mut scanning = false;
    let mut scan_cancel: Option<watch::Sender<bool>> = None;
    let mut next = next_slot(Local::now());
    let (done_tx, done_rx) = flume::unbounded::<()>();
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
            _ = done_rx.recv_async(), if scanning => { scanning = false; scan_cancel = None; }
            command = commands.recv_async() => match command {
                Ok(Command::Scan { mode }) => {
                    if scanning { emit_error(&events, ErrorKind::Busy, "a scan is already running".into(), "scan"); continue; }
                    match acquire_scan_lock() {
                        Ok(lock) => {
                            scanning = true;
                            let (cancel_tx, cancel_rx) = watch::channel(false);
                            scan_cancel = Some(cancel_tx);
                            let p = pool.clone(); let tx = events.clone(); let done = done_tx.clone();
                            let path = db_path.clone();
                            let cfg_count = read_config_files();
                            let _ = events.send(EngineEvent::ScanStatus(ScanState::Running { mode }));
                            tokio::spawn(async move { run_scan(ScanJob { pool: p, db_path: path, mode, lock, events: tx, done, cancel: cancel_rx, config_files: cfg_count }).await; });
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => emit_error(&events, ErrorKind::Busy, "another process holds the scan lock".into(), "scan"),
                        Err(e) => emit_error(&events, ErrorKind::General, e.to_string(), "scan"),
                    }
                }
                Ok(Command::CancelScan) => { if let Some(tx) = &scan_cancel { let _ = tx.send(true); } }
                Ok(Command::RefreshConfig) => { let count = read_config_files(); emit_dashboard(&events, snapshot(&pool, &db_path, count).await); }
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
    done: flume::Sender<()>,
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
            None => { if let Some(parent) = db_path.parent() { fs::create_dir_all(parent)?; } let p = connect(&db_path).await?; migrate(&p).await?; p }
        };
        let insert = sqlx::query("INSERT INTO scan_runs(mode,status,started_at,matched,stats_json) VALUES(?, 'running', ?, 0, '{\"stub\":true}')")
            .bind(match mode { ScanMode::Quick => "quick", ScanMode::Full => "full" }).bind(&started).execute(&pool).await?;
        let run_id = insert.last_insert_rowid();
        let pause = tokio::time::sleep(StdDuration::from_millis(900)); tokio::pin!(pause);
        let cancelled = tokio::select! { _ = &mut pause => false, changed = cancel.changed() => changed.is_ok() && *cancel.borrow() };
        let status = if cancelled { "cancelled" } else { "ok" };
        sqlx::query("UPDATE scan_runs SET status = ?, finished_at = ?, matched = 0, stats_json = '{\"stub\":true}' WHERE id = ?")
            .bind(status).bind(Local::now().to_rfc3339()).bind(run_id).execute(&pool).await?;
        emit_dashboard(&events, snapshot(&Some(pool), &db_path, config_files).await);
        Ok::<bool, anyhow::Error>(cancelled)
    }.await;
    match result {
        Ok(cancelled) => {
            let _ = events.send(EngineEvent::ScanStatus(ScanState::Idle));
            if cancelled {
                let _ = events.send(EngineEvent::Error {
                    kind: ErrorKind::General,
                    message: "scan cancelled".into(),
                    source: "scan".into(),
                });
            }
        }
        Err(error) => {
            let _ = events.send(EngineEvent::ScanStatus(ScanState::Error(error.to_string())));
            emit_error(&events, ErrorKind::General, error.to_string(), "scan");
        }
    }
    drop(lock);
    let _ = done.send(());
}

async fn connect(path: &std::path::Path) -> Result<SqlitePool> {
    let url = format!("sqlite://{}?mode=rwc", path.to_string_lossy());
    Ok(SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await?)
}
async fn migrate(pool: &SqlitePool) -> Result<()> {
    sqlx::query("CREATE TABLE IF NOT EXISTS postings(id TEXT PRIMARY KEY, retired_at TEXT);")
        .execute(pool)
        .await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS seen(id TEXT PRIMARY KEY);")
        .execute(pool)
        .await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS scan_runs(id INTEGER PRIMARY KEY AUTOINCREMENT, mode TEXT NOT NULL, status TEXT NOT NULL, started_at TEXT NOT NULL, finished_at TEXT, matched INTEGER, stats_json TEXT NOT NULL DEFAULT '{}');").execute(pool).await?;
    Ok(())
}
async fn snapshot(
    pool: &Option<SqlitePool>,
    path: &std::path::Path,
    config_files: usize,
) -> DashboardSnapshot {
    let mut out = DashboardSnapshot {
        live_postings: 0,
        seen_count: 0,
        last_scan: None,
        database_path: path.display().to_string(),
        has_database: pool.is_some(),
        config_files,
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
        let path = std::env::temp_dir().join(format!("jh-engine-{}.sqlite", std::process::id()));
        let _ = fs::remove_file(&path);
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
        let query = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let count: i64 = query.block_on(async {
            sqlx::query("SELECT COUNT(*) FROM scan_runs")
                .fetch_one(&connect(&path).await.unwrap())
                .await
                .unwrap()
                .get(0)
        });
        assert_eq!(count, 1);
        handle.shutdown().unwrap();
        let _ = fs::remove_file(path);
    }
    #[test]
    fn busy_lock() {
        let _serial = SCAN_TEST_LOCK.lock().unwrap();
        let file = acquire_scan_lock().unwrap();
        let path =
            std::env::temp_dir().join(format!("jh-engine-busy-{}.sqlite", std::process::id()));
        let _ = fs::remove_file(&path);
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
        let _ = fs::remove_file(path);
    }
}
