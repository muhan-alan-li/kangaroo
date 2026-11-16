use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, Duration, Local, NaiveTime, SecondsFormat, Utc, Weekday};
use serde_json::{Value, json};
use sqlx::{Row, SqlitePool};
use std::{
    fs,
    path::PathBuf,
    thread::{self, JoinHandle},
    time::Duration as StdDuration,
};
use tokio::sync::watch;

pub use jh_protocol::{
    BriefDetail, Command, DashboardSnapshot, EngineEvent, ErrorKind, LastScan, PostingDetail,
    PostingRow, ScanMode, ScanState, Settings,
};

mod descriptions;
mod scan;

pub use scan::{ScanOutcome, run as run_scan_once};
pub mod boardstate;

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
    let config_files = read_config_files();
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
                Ok(Command::MakeBrief { posting_id }) => {
                    spawn_brief(&pool, &db_path, config_files, &events, posting_id);
                }
                Ok(Command::PostingDetail { posting_id }) => {
                    if let Some(pool) = &pool
                        && let Some(detail) = posting_detail(pool, &posting_id).await
                    {
                        let _ = events.send(EngineEvent::PostingDetail(detail));
                    }
                }
                Ok(Command::MakeBriefs { posting_ids }) => {
                    for posting_id in posting_ids {
                        let existing = match &pool {
                            Some(pool) => sqlx::query(
                                "SELECT status FROM briefs WHERE posting_id = ?",
                            )
                            .bind(&posting_id)
                            .fetch_optional(pool)
                            .await
                            .ok()
                            .flatten()
                            .and_then(|row| row.try_get::<String, _>("status").ok()),
                            None => None,
                        };
                        if matches!(existing.as_deref(), Some("ok") | Some("running")) {
                            let _ = events.send(EngineEvent::BriefUpdated {
                                posting_id,
                                state: "skipped".into(),
                            });
                            continue;
                        }
                        spawn_brief(&pool, &db_path, config_files, &events, posting_id);
                    }
                }
                Ok(Command::Apply { posting_id }) => {
                    if let Some(pool) = &pool {
                        apply(pool, &posting_id).await;
                        emit_dashboard(
                            &events,
                            snapshot(&Some(pool.clone()), &db_path, config_files, scan_state.clone())
                                .await,
                        );
                    }
                }
                Ok(Command::Unapply { posting_ids }) => {
                    if let Some(pool) = &pool {
                        unapply(pool, &posting_ids).await;
                        emit_dashboard(
                            &events,
                            snapshot(&Some(pool.clone()), &db_path, config_files, scan_state.clone())
                                .await,
                        );
                    }
                }
                Ok(Command::SaveSettings { settings }) => {
                    if let Some(pool) = &pool {
                        let mut values = settings.values.clone();
                        if let Some(key) = values
                            .get("deepseek_api_key")
                            .and_then(Value::as_str)
                            .filter(|key| !key.is_empty())
                            .map(str::to_owned)
                        {
                            let stored = std::process::Command::new("security")
                                .args([
                                    "add-generic-password",
                                    "-U",
                                    "-s",
                                    "com.muhanli.jobhunt.deepseek",
                                    "-a",
                                    "jobhunt",
                                    "-w",
                                    &key,
                                ])
                                .status()
                                .is_ok_and(|status| status.success());
                            if !stored {
                                emit_error(
                                    &events,
                                    ErrorKind::General,
                                    "could not store the model key in the Keychain".into(),
                                    "settings",
                                );
                            }
                            if let Some(object) = values.as_object_mut() {
                                object.remove("deepseek_api_key");
                            }
                        }
                        let stamp = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
                        let _ = sqlx::query(
                            "INSERT INTO settings(key,value_json,updated_at) VALUES('app',?,?)
                             ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json,
                                updated_at=excluded.updated_at",
                        )
                        .bind(values.to_string())
                        .bind(stamp)
                        .execute(pool)
                        .await;
                        emit_dashboard(
                            &events,
                            snapshot(&Some(pool.clone()), &db_path, config_files, scan_state.clone())
                                .await,
                        );
                    }
                }
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
        cancel,
        config_files,
    } = job;
    let result = async {
        let pool = match pool {
            Some(pool) => pool,
            None => open_database(&db_path).await?,
        };
        let mode = match mode {
            ScanMode::Quick => "quick",
            ScanMode::Full => "full",
        };
        let outcome = scan::run(&pool, mode, &events, &cancel, &[]).await?;
        emit_dashboard(
            &events,
            snapshot(&Some(pool), &db_path, config_files, ScanState::Idle).await,
        );
        Ok::<usize, anyhow::Error>(outcome.matched)
    }
    .await;
    match result {
        Ok(_) => {
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
        open: Vec::new(),
        shortlisted: Vec::new(),
        applied: Vec::new(),
        briefs_enabled: briefs_enabled(),
        board_total: 0,
        board_ok: 0,
        board_failing: 0,
        board_broken: 0,
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

        let open = sqlx::query(
            "SELECT p.id,p.company,p.title,p.location,p.url,p.source,p.ats,p.tier,p.score,
                p.age_days,p.posted,p.expires_on,p.has_text,b.status AS brief_status
             FROM postings p LEFT JOIN briefs b ON b.posting_id = p.id
             WHERE p.retired_at IS NULL
               AND (b.status IS NULL OR b.status = 'failed')
               AND NOT EXISTS (SELECT 1 FROM applied a WHERE a.posting_id = p.id)
             ORDER BY (p.score IS NULL), p.score DESC, (p.age_days IS NULL), p.age_days ASC",
        )
        .fetch_all(pool)
        .await
        .unwrap_or_default();
        out.open = open.iter().map(posting_row).collect();

        let shortlisted = sqlx::query(
            "SELECT p.id,p.company,p.title,p.location,p.url,p.source,p.ats,p.tier,p.score,
                p.age_days,p.posted,p.expires_on,p.has_text,b.status AS brief_status
             FROM postings p JOIN briefs b ON b.posting_id = p.id
             WHERE b.status IN ('ok','running')
               AND NOT EXISTS (SELECT 1 FROM applied a WHERE a.posting_id = p.id)
             ORDER BY (b.status = 'running') DESC, (p.score IS NULL), p.score DESC",
        )
        .fetch_all(pool)
        .await
        .unwrap_or_default();
        out.shortlisted = shortlisted.iter().map(posting_row).collect();

        let applied = sqlx::query(
            "SELECT posting_id,applied_at,snapshot_json FROM applied ORDER BY applied_at DESC",
        )
        .fetch_all(pool)
        .await
        .unwrap_or_default();
        out.applied = applied.iter().filter_map(applied_row).collect();

        for row in sqlx::query("SELECT status, COUNT(*) AS n FROM board_health GROUP BY status")
            .fetch_all(pool)
            .await
            .unwrap_or_default()
        {
            let n: i64 = row.get("n");
            out.board_total += n;
            match row.get::<Option<String>, _>("status").as_deref() {
                Some("ok") => out.board_ok += n,
                Some("broken") => out.board_broken += n,
                _ => out.board_failing += n,
            }
        }
    }
    out
}

fn posting_row(row: &sqlx::sqlite::SqliteRow) -> PostingRow {
    let age_days: Option<i64> = row.get("age_days");
    let posted: Option<String> = row.get::<Option<String>, _>("posted").or_else(|| {
        age_days.map(|age| {
            (Utc::now().date_naive() - Duration::days(age))
                .format("%Y-%m-%d")
                .to_string()
        })
    });
    PostingRow {
        id: row.get("id"),
        company: row.get("company"),
        title: row.get("title"),
        location: row.get("location"),
        url: row.get("url"),
        source: row.get("source"),
        ats: row.get("ats"),
        tier: row.get("tier"),
        score: row.get("score"),
        age_days,
        posted,
        expires_on: row.get("expires_on"),
        brief_status: row.get("brief_status"),
        has_text: row.get::<i64, _>("has_text") != 0,
        applied_at: None,
    }
}

fn applied_row(row: &sqlx::sqlite::SqliteRow) -> Option<PostingRow> {
    let snapshot: Value =
        serde_json::from_str(row.get::<String, _>("snapshot_json").as_str()).ok()?;
    let text = |key: &str| snapshot.get(key).and_then(Value::as_str).map(str::to_owned);
    Some(PostingRow {
        id: text("id").or_else(|| Some(row.get::<String, _>("posting_id")))?,
        company: text("company").unwrap_or_default(),
        title: text("title").unwrap_or_default(),
        location: text("location"),
        url: text("url"),
        source: text("source").unwrap_or_default(),
        ats: text("ats"),
        tier: text("tier"),
        score: snapshot.get("score").and_then(Value::as_i64),
        age_days: snapshot.get("age_days").and_then(Value::as_i64),
        posted: text("posted"),
        expires_on: text("expires_on"),
        brief_status: text("brief_status"),
        has_text: snapshot
            .get("has_text")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        applied_at: Some(row.get::<String, _>("applied_at")),
    })
}

/// One posting with everything the drawer draws: the table's columns, the
/// cached text, and the brief when one exists.
async fn posting_detail(pool: &SqlitePool, posting_id: &str) -> Option<PostingDetail> {
    let row = sqlx::query(
        "SELECT p.id,p.company,p.title,p.location,p.url,p.source,p.ats,p.tier,p.score,
            p.age_days,p.posted,p.expires_on,p.has_text,b.status AS brief_status,
            p.raw_id,p.salary_raw,p.keywords_json,p.skills_have_json,p.skills_gap_json,
            p.emails_json,p.level_ii,p.requires_us_work_auth,p.below_comp_floor,
            p.first_seen_at,p.last_seen_at
         FROM postings p LEFT JOIN briefs b ON b.posting_id = p.id
         WHERE p.id = ?",
    )
    .bind(posting_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()?;

    let description = sqlx::query("SELECT text, source FROM descriptions WHERE posting_id = ?")
        .bind(posting_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    let text = description
        .as_ref()
        .and_then(|row| row.get::<Option<String>, _>("text"))
        .filter(|text| !text.trim().is_empty());

    let brief = sqlx::query(
        "SELECT status,engine,generated_at,payload_json,error FROM briefs WHERE posting_id = ?",
    )
    .bind(posting_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .map(|row| BriefDetail {
        status: row.get("status"),
        engine: row.get("engine"),
        generated_at: row.get("generated_at"),
        error: row.get("error"),
        payload: row
            .get::<Option<String>, _>("payload_json")
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or(Value::Null),
    });

    Some(PostingDetail {
        row: posting_row(&row),
        raw_id: row.get("raw_id"),
        salary_raw: row.get("salary_raw"),
        keywords: string_list(row.get("keywords_json")),
        skills_have: string_list(row.get("skills_have_json")),
        skills_gap: string_list(row.get("skills_gap_json")),
        emails: string_list(row.get("emails_json")),
        level_ii: row.get::<i64, _>("level_ii") != 0,
        requires_us_work_auth: row.get::<i64, _>("requires_us_work_auth") != 0,
        below_comp_floor: row.get::<i64, _>("below_comp_floor") != 0,
        first_seen_at: row.get("first_seen_at"),
        last_seen_at: row.get("last_seen_at"),
        description: text,
        description_source: description.as_ref().and_then(|row| row.get("source")),
        brief,
    })
}

/// A `JSON` text column that holds a list of strings.
fn string_list(raw: Option<String>) -> Vec<String> {
    raw.and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn briefs_enabled() -> bool {
    if std::env::var("DEEPSEEK_API_KEY").is_ok_and(|key| !key.is_empty()) {
        return true;
    }
    let keychain = std::process::Command::new("security")
        .args([
            "find-generic-password",
            "-s",
            "com.muhanli.jobhunt.deepseek",
            "-w",
        ])
        .output();
    if keychain.is_ok_and(|output| output.status.success()) {
        return true;
    }
    std::env::var("HOME")
        .ok()
        .map(|home| {
            std::path::PathBuf::from(home)
                .join(".hermes")
                .join("auth.json")
        })
        .is_some_and(|path| path.exists())
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

fn spawn_brief(
    pool: &Option<SqlitePool>,
    db_path: &std::path::Path,
    config_files: usize,
    events: &flume::Sender<EngineEvent>,
    posting_id: String,
) {
    let Some(pool) = pool.clone() else {
        emit_error(
            events,
            ErrorKind::General,
            "the database is not open".into(),
            "brief",
        );
        return;
    };
    let events = events.clone();
    let db_path = db_path.to_path_buf();
    tokio::spawn(async move {
        let root = std::env::var_os("JH_ROOT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| ".".into()));
        let result = jh_brief::generate(&pool, &root, &posting_id).await;
        let state = result
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("failed")
            .to_owned();
        let _ = events.send(EngineEvent::BriefUpdated { posting_id, state });
        emit_dashboard(
            &events,
            snapshot(&Some(pool), &db_path, config_files, ScanState::Idle).await,
        );
    });
}

async fn apply(pool: &SqlitePool, posting_id: &str) {
    let Ok(Some(row)) = sqlx::query(
        "SELECT id,source,ats,company,title,location,url,posted,age_days,tier,score
         FROM postings WHERE id = ?",
    )
    .bind(posting_id)
    .fetch_optional(pool)
    .await
    else {
        return;
    };
    let stamp = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    let snapshot = json!({
        "id": row.get::<String, _>("id"),
        "source": row.get::<String, _>("source"),
        "ats": row.get::<Option<String>, _>("ats"),
        "company": row.get::<String, _>("company"),
        "title": row.get::<String, _>("title"),
        "location": row.get::<Option<String>, _>("location"),
        "url": row.get::<Option<String>, _>("url"),
        "posted": row.get::<Option<String>, _>("posted"),
        "age_days": row.get::<Option<i64>, _>("age_days"),
        "tier": row.get::<Option<String>, _>("tier"),
        "score": row.get::<Option<i64>, _>("score"),
        "applied_at": stamp,
    });
    let _ = sqlx::query(
        "INSERT INTO applied(posting_id,applied_at,snapshot_json) VALUES(?,?,?)
         ON CONFLICT(posting_id) DO UPDATE SET applied_at=excluded.applied_at,
            snapshot_json=excluded.snapshot_json",
    )
    .bind(posting_id)
    .bind(&stamp)
    .bind(snapshot.to_string())
    .execute(pool)
    .await;
}

async fn unapply(pool: &SqlitePool, posting_ids: &[String]) {
    let Ok(mut tx) = pool.begin().await else {
        return;
    };
    for posting_id in posting_ids {
        let _ = sqlx::query("DELETE FROM applied WHERE posting_id = ?")
            .bind(posting_id)
            .execute(&mut *tx)
            .await;
    }
    let _ = tx.commit().await;
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

    fn seed_posting(path: &std::path::Path, id: &str, company: &str, title: &str) {
        block_on(async {
            let pool = open_database(path).await.unwrap();
            sqlx::query(
                "INSERT INTO postings(id,source,company,title,location,url,first_seen_at,last_seen_at)
                 VALUES(?, 'jobbank', ?, ?, 'Vancouver, BC', 'https://example.com', '2026-11-13T00:00:00Z', '2026-11-13T00:00:00Z')",
            )
            .bind(id)
            .bind(company)
            .bind(title)
            .execute(&pool)
            .await
            .unwrap();
            pool.close().await;
        });
    }

    #[test]
    fn dashboard_splits_the_three_stages() {
        let _serial = SCAN_TEST_LOCK.lock().unwrap();
        let path = temp_path("stages");
        remove_database(&path);
        seed_posting(&path, "aaaaaaaaaaaaaaaa", "Open Co", "Junior Engineer");
        seed_posting(&path, "bbbbbbbbbbbbbbbb", "Brief Co", "Junior Engineer");
        seed_posting(&path, "cccccccccccccccc", "Failed Co", "Junior Engineer");
        seed_posting(&path, "dddddddddddddddd", "Applied Co", "Junior Engineer");
        block_on(async {
            let pool = open_database(&path).await.unwrap();
            sqlx::query(
                "INSERT INTO briefs(posting_id,status,engine,payload_json)
                 VALUES('bbbbbbbbbbbbbbbb','ok','deepseek','{}'),
                        ('cccccccccccccccc','failed','deepseek','{}')",
            )
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO applied(posting_id,applied_at,snapshot_json)
                 VALUES('dddddddddddddddd','2026-11-13T00:00:00Z',
                        '{\"id\":\"dddddddddddddddd\",\"company\":\"Applied Co\",\"title\":\"Junior Engineer\",\"source\":\"jobbank\"}')",
            )
            .execute(&pool)
            .await
            .unwrap();
            pool.close().await;
        });

        let handle = Engine::spawn(path.clone()).unwrap();
        let rx = handle.events.clone();
        handle.commands.send(Command::Snapshot).unwrap();
        let snapshot = loop {
            if let EngineEvent::Dashboard(snapshot) =
                rx.recv_timeout(StdDuration::from_secs(5)).unwrap()
            {
                break snapshot;
            }
        };
        assert_eq!(
            snapshot
                .open
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            vec!["aaaaaaaaaaaaaaaa", "cccccccccccccccc"]
        );
        assert_eq!(
            snapshot
                .shortlisted
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            vec!["bbbbbbbbbbbbbbbb"]
        );
        assert_eq!(
            snapshot
                .applied
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            vec!["dddddddddddddddd"]
        );
        handle.shutdown().unwrap();
        remove_database(&path);
    }

    #[test]
    fn apply_and_unapply_move_a_posting() {
        let _serial = SCAN_TEST_LOCK.lock().unwrap();
        let path = temp_path("apply");
        remove_database(&path);
        seed_posting(&path, "eeeeeeeeeeeeeeee", "Apply Co", "Junior Engineer");
        let handle = Engine::spawn(path.clone()).unwrap();
        let rx = handle.events.clone();
        let _ = next_dashboard(&rx);
        handle
            .commands
            .send(Command::Apply {
                posting_id: "eeeeeeeeeeeeeeee".into(),
            })
            .unwrap();
        let after_apply = next_dashboard(&rx);
        assert_eq!(after_apply.applied.len(), 1);
        assert!(after_apply.open.is_empty());
        handle
            .commands
            .send(Command::Unapply {
                posting_ids: vec!["eeeeeeeeeeeeeeee".into()],
            })
            .unwrap();
        let after_unapply = next_dashboard(&rx);
        assert!(after_unapply.applied.is_empty());
        assert_eq!(after_unapply.open.len(), 1);
        handle.shutdown().unwrap();
        remove_database(&path);
    }

    /// The drawer asks for one posting and gets its cached text and, when one
    /// exists, the brief's payload. A posting with neither still answers, so the
    /// panel can say so instead of spinning.
    #[test]
    fn posting_detail_carries_the_text_and_the_brief() {
        let _serial = SCAN_TEST_LOCK.lock().unwrap();
        let path = temp_path("detail");
        remove_database(&path);
        seed_posting(&path, "1111111111111111", "Detail Co", "Junior Engineer");
        block_on(async {
            let pool = open_database(&path).await.unwrap();
            sqlx::query(
                "INSERT INTO descriptions(posting_id,text,source)
                 VALUES('1111111111111111','Build reliable systems.','cache')",
            )
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO briefs(posting_id,status,engine,generated_at,payload_json)
                 VALUES('1111111111111111','ok','deepseek','2026-11-13T12:00:00Z',
                        '{\"verdict\":\"Apply.\",\"key_skills\":[\"Rust\"]}')",
            )
            .execute(&pool)
            .await
            .unwrap();
            pool.close().await;
        });

        let handle = Engine::spawn(path.clone()).unwrap();
        let rx = handle.events.clone();
        let _ = next_dashboard(&rx);
        handle
            .commands
            .send(Command::PostingDetail {
                posting_id: "1111111111111111".into(),
            })
            .unwrap();
        let detail = loop {
            match rx.recv_timeout(StdDuration::from_secs(5)).unwrap() {
                EngineEvent::PostingDetail(detail) => break detail,
                _ => continue,
            }
        };
        assert_eq!(detail.row.company, "Detail Co");
        assert_eq!(
            detail.description.as_deref(),
            Some("Build reliable systems.")
        );
        assert_eq!(detail.description_source.as_deref(), Some("cache"));
        let brief = detail.brief.expect("the brief row was not in the detail");
        assert_eq!(brief.status, "ok");
        assert_eq!(brief.payload["verdict"], "Apply.");

        // An id the database does not know answers with nothing at all, so the
        // app never waits on a detail that cannot arrive.
        handle
            .commands
            .send(Command::PostingDetail {
                posting_id: "nothing".into(),
            })
            .unwrap();
        handle.commands.send(Command::Snapshot).unwrap();
        assert!(matches!(
            rx.recv_timeout(StdDuration::from_secs(5)).unwrap(),
            EngineEvent::Dashboard(_)
        ));
        handle.shutdown().unwrap();
        remove_database(&path);
    }
}
