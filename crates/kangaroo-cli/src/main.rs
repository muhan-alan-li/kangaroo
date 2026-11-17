use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use sqlx::{Row, SqlitePool};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Parser)]
#[command(name = "jh")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}
#[derive(Subcommand)]
enum Commands {
    Import {
        #[arg(long = "from")]
        from: PathBuf,
        #[arg(long)]
        db: PathBuf,
    },
    /// Run one scan against the configured sources and write the database.
    Scan {
        #[arg(long)]
        db: PathBuf,
        /// Limit to one or more source ids from data/config/sources.yaml.
        #[arg(long = "source")]
        source: Vec<String>,
        /// Print the outcome as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Sweep the Workday boards into the cache the daily scan reads.
    Sweep {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        workers: Option<usize>,
        #[arg(long)]
        limit: Option<usize>,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        json: bool,
    },
    /// Re-check the Workday boards whose backoff has come up.
    Retry {
        #[arg(long)]
        db: PathBuf,
        #[arg(long, default_value_t = 8)]
        workers: usize,
        #[arg(long)]
        limit: Option<usize>,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        json: bool,
    },
    /// Remove structurally dead Workday boards from the config and the health table.
    Prune {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        dry_run: bool,
        /// Remove one board by tenant|wd|site (repeatable).
        #[arg(long = "key")]
        key: Vec<String>,
    },
    /// Probe the Greenhouse/Ashby/Lever/Workable boards and rewrite ats_boards.yaml.
    ProbeAts {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        report: bool,
        #[arg(long, default_value_t = 1)]
        min_jobs: usize,
        #[arg(long, default_value_t = 10)]
        workers: usize,
    },
    /// Rebuild a board list from the links in the Simplify feed.
    Harvest {
        #[arg(long)]
        db: PathBuf,
        #[arg(value_enum)]
        kind: HarvestKind,
        #[arg(long)]
        report: bool,
    },
    /// Re-run location tiering on every stored posting and applied row with the
    /// current data/config/filters.yaml, so existing data fits the categories.
    MigrateLocations {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum HarvestKind {
    Workday,
    Jsonboards,
}
#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Import { from, db } => {
            if let Some(parent) = db.parent().filter(|p| !p.as_os_str().is_empty()) {
                fs::create_dir_all(parent)?;
            }
            let pool =
                kangaroo_db::connect(db.to_str().context("database path is not UTF-8")?).await?;
            kangaroo_db::migrate(&pool).await?;
            import_all(&pool, &from).await?;
        }
        Commands::Scan { db, source, json } => {
            if let Some(parent) = db.parent().filter(|p| !p.as_os_str().is_empty()) {
                fs::create_dir_all(parent)?;
            }
            let pool =
                kangaroo_db::connect(db.to_str().context("database path is not UTF-8")?).await?;
            kangaroo_db::migrate(&pool).await?;
            let (events, _event_rx) = flume::unbounded();
            let (_cancel_tx, cancel) = tokio::sync::watch::channel(false);
            let outcome =
                kangaroo_engine::run_scan_once(&pool, "full", &events, &cancel, &source).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&outcome)?);
            } else {
                println!(
                    "collected={} matched={} sources={:?}",
                    outcome.collected, outcome.matched, outcome.per_source
                );
                println!("seconds={:?}", outcome.source_seconds);
                println!("tiers={:?}", outcome.tier_counts);
                if !outcome.dropped.is_empty() {
                    println!("dropped={:?}", outcome.dropped);
                }
            }
        }
        Commands::Sweep {
            db,
            workers,
            limit,
            dry_run,
            json,
        } => run_sweep(db, workers, limit, dry_run, json).await?,
        Commands::Retry {
            db,
            workers,
            limit,
            force,
            dry_run,
            json,
        } => run_retry(db, workers, limit, force, dry_run, json).await?,
        Commands::Prune { db, dry_run, key } => run_prune(db, dry_run, key).await?,
        Commands::ProbeAts {
            db,
            report,
            min_jobs,
            workers,
        } => run_probe(db, report, min_jobs, workers).await?,
        Commands::Harvest { db, kind, report } => run_harvest(db, kind, report).await?,
        Commands::MigrateLocations { db, dry_run } => run_migrate_locations(db, dry_run).await?,
    }
    Ok(())
}

fn open_pool(db: &Path) -> Result<SqlitePool> {
    if let Some(parent) = db.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    Ok(SqlitePool::connect_lazy(&format!(
        "sqlite://{}?mode=rwc",
        db.display()
    ))?)
}

fn read_yaml(path: &Path) -> Result<serde_yaml::Value> {
    Ok(serde_yaml::from_str(&fs::read_to_string(path)?)?)
}

fn ats_source_config() -> Result<serde_yaml::Value> {
    let sources = read_yaml(Path::new("data/config/sources.yaml"))?;
    sources
        .get("sources")
        .and_then(serde_yaml::Value::as_sequence)
        .and_then(|sources| {
            sources.iter().find(|source| {
                source.get("kind").and_then(serde_yaml::Value::as_str) == Some("ats")
            })
        })
        .cloned()
        .context("data/config/sources.yaml has no ats source")
}

fn scan_context() -> Result<kangaroo_scan::ScanContext> {
    let http = kangaroo_scan::HttpClient::from_sources_file(Path::new("data/config/sources.yaml"))?;
    Ok(kangaroo_scan::ScanContext::with_root(http, "."))
}

fn workday_boards(
    ctx: &kangaroo_scan::ScanContext,
    cfg: &serde_yaml::Value,
) -> Vec<kangaroo_scan::sources::ats::Board> {
    kangaroo_scan::sources::ats::load_entries(ctx, cfg)
        .into_iter()
        .filter(|board| {
            board.ats == "workday" && !board.tenant.is_empty() && !board.site.is_empty()
        })
        .collect()
}

fn with_workers(mut cfg: serde_yaml::Value, workers: Option<usize>) -> serde_yaml::Value {
    if let Some(workers) = workers
        && let Some(workday) = cfg
            .get_mut("workday")
            .and_then(|value| value.as_mapping_mut())
    {
        workday.insert(
            serde_yaml::Value::String("workers".into()),
            serde_yaml::Value::Number(serde_yaml::Number::from(workers as u64)),
        );
    }
    cfg
}

async fn run_sweep(
    db: PathBuf,
    workers: Option<usize>,
    limit: Option<usize>,
    dry_run: bool,
    json: bool,
) -> Result<()> {
    let pool = open_pool(&db)?;
    kangaroo_db::migrate(&pool).await?;
    let cfg = with_workers(ats_source_config()?, workers);
    let ctx = scan_context()?;
    let all = workday_boards(&ctx, &cfg);
    let partial = limit.is_some_and(|limit| all.len() > limit);
    let batch = match limit {
        Some(limit) => all.iter().take(limit).cloned().collect::<Vec<_>>(),
        None => all.clone(),
    };
    if batch.is_empty() {
        anyhow::bail!("no Workday boards configured");
    }
    eprintln!("[workday] sweeping {} boards", batch.len());
    let started = std::time::Instant::now();
    let (postings, _dead) = kangaroo_scan::sources::workday::fetch_boards(&ctx, &batch, &cfg).await;
    let wall = started.elapsed().as_secs_f64();
    let notes = ctx.take_boards();
    let previous = kangaroo_engine::boardstate::read_sweep(&pool, f64::MAX)
        .await
        .ok()
        .flatten()
        .map(|postings| postings.len())
        .unwrap_or(0);
    let (trusted, reason) = kangaroo_engine::boardstate::sweep_verdict(&notes, previous);
    if !trusted {
        eprintln!("[workday] REFUSED this sweep: {reason}");
        eprintln!("[workday] nothing written; the previous cache and board state are unchanged");
        std::process::exit(2);
    }
    if !dry_run {
        kangaroo_engine::boardstate::write_notes(&pool, &notes).await?;
        if partial {
            eprintln!("[workday] partial sweep: health recorded, cache left as it was");
        } else {
            kangaroo_engine::boardstate::write_sweep(&pool, &postings).await?;
        }
    }
    let (known, ok, failing, broken) = kangaroo_engine::boardstate::summary(&pool).await?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "boards": batch.len(),
                "postings": postings.len(),
                "wall_seconds": (wall * 10.0).round() / 10.0,
                "dry_run": dry_run,
                "board_state": {"known": known, "ok": ok, "failing": failing, "broken": broken},
            }))?
        );
    } else {
        println!(
            "workday sweep: {} boards in {wall:.1}s -> {} postings",
            batch.len(),
            postings.len()
        );
        println!("  {known} known, {ok} ok, {failing} failing, {broken} broken");
        if dry_run {
            println!("  dry run: no cache and no health written");
        }
    }
    Ok(())
}

async fn run_retry(
    db: PathBuf,
    workers: usize,
    limit: Option<usize>,
    force: bool,
    dry_run: bool,
    json: bool,
) -> Result<()> {
    let pool = open_pool(&db)?;
    kangaroo_db::migrate(&pool).await?;
    let cfg = with_workers(ats_source_config()?, Some(workers));
    let ctx = scan_context()?;
    let configured = workday_boards(&ctx, &cfg);
    let by_key: std::collections::HashMap<String, _> = configured
        .iter()
        .cloned()
        .map(|board| {
            (
                format!("{}|{}|{}", board.tenant, board.wd, board.site),
                board,
            )
        })
        .collect();
    let mut due = kangaroo_engine::boardstate::due(&pool, force).await?;
    due.retain(|row| by_key.contains_key(&row.key));
    if let Some(limit) = limit {
        due.truncate(limit);
    }
    let (known, ok, failing, broken) = kangaroo_engine::boardstate::summary(&pool).await?;
    eprintln!("[retry] {known} known, {ok} ok, {failing} failing, {broken} broken");
    if due.is_empty() {
        eprintln!("[retry] nothing is due");
        return Ok(());
    }
    if dry_run {
        for row in &due {
            eprintln!(
                "[retry] would check {} ({}, last: {})",
                row.key,
                row.status.as_deref().unwrap_or("unknown"),
                row.last_error.as_deref().unwrap_or("none")
            );
        }
        eprintln!("[retry] {} board(s) due, dry run", due.len());
        return Ok(());
    }
    let batch: Vec<_> = due
        .iter()
        .filter_map(|row| by_key.get(&row.key).cloned())
        .collect();
    let started = std::time::Instant::now();
    let (postings, _dead) = kangaroo_scan::sources::workday::fetch_boards(&ctx, &batch, &cfg).await;
    let wall = started.elapsed().as_secs_f64();
    let notes = ctx.take_boards();
    let failed = notes.iter().filter(|note| !note.ok).count();
    if !notes.is_empty() && failed >= 25 && (failed as f64 / notes.len() as f64) > 0.999 {
        eprintln!("[retry] not recording this run: every board failed; board state is unchanged");
        std::process::exit(2);
    }
    kangaroo_engine::boardstate::write_notes(&pool, &notes).await?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "checked": notes.len(),
                "recovered": notes.iter().filter(|n| n.ok).count(),
                "still_failing": failed,
                "postings": postings.len(),
                "wall_seconds": (wall * 10.0).round() / 10.0,
            }))?
        );
    } else {
        println!(
            "retry pass: {} checked in {wall:.1}s, {} recovered, {failed} still failing",
            notes.len(),
            notes.len() - failed
        );
    }
    Ok(())
}

async fn run_prune(db: PathBuf, dry_run: bool, explicit: Vec<String>) -> Result<()> {
    let pool = open_pool(&db)?;
    kangaroo_db::migrate(&pool).await?;
    let rows = sqlx::query(
        "SELECT key,last_error FROM board_health WHERE status='broken' AND error_kind='structural'",
    )
    .fetch_all(&pool)
    .await?;
    let mut targets: std::collections::BTreeMap<String, String> = rows
        .into_iter()
        .map(|row| {
            (
                row.get::<String, _>("key"),
                row.get::<Option<String>, _>("last_error")
                    .unwrap_or_else(|| "structural failure".into()),
            )
        })
        .collect();
    for key in explicit {
        targets.entry(key).or_insert_with(|| "requested".into());
    }
    if targets.is_empty() {
        eprintln!("[prune] no structural-broken boards; nothing to remove");
        return Ok(());
    }
    let config = Path::new("data/config/workday_boards.yaml");
    let text = fs::read_to_string(config)?;
    let keyed = regex::Regex::new(r#"\b(tenant|wd|site):\s*"?([^",}\s]+)"?"#).unwrap();
    let mut kept = Vec::new();
    let mut removed = Vec::new();
    for line in text.lines() {
        let fields: std::collections::HashMap<&str, &str> = keyed
            .captures_iter(line)
            .filter_map(|capture| Some((capture.get(1)?.as_str(), capture.get(2)?.as_str())))
            .collect();
        if line.trim_start().starts_with('-')
            && fields.contains_key("tenant")
            && fields.contains_key("wd")
            && fields.contains_key("site")
        {
            let norm = |value: &str| -> String {
                value
                    .parse::<i64>()
                    .map(|number| number.to_string())
                    .unwrap_or_else(|_| value.to_owned())
            };
            let key = format!(
                "{}|{}|{}",
                norm(fields["tenant"]),
                norm(fields["wd"]),
                norm(fields["site"])
            );
            if targets.contains_key(&key) {
                removed.push(key);
                continue;
            }
        }
        kept.push(line);
    }
    if removed.is_empty() {
        eprintln!("[prune] nothing matched in the config; no change");
        return Ok(());
    }
    if dry_run {
        for key in &removed {
            eprintln!(
                "[prune] would remove {key} ({})",
                targets.get(key).map(String::as_str).unwrap_or("requested")
            );
        }
        eprintln!(
            "[prune] {} board(s) would be removed; dry run",
            removed.len()
        );
        return Ok(());
    }
    atomic_write(config, &(kept.join("\n") + "\n"))?;
    write_tombstones(&removed, &targets)?;
    for key in &removed {
        sqlx::query("DELETE FROM board_health WHERE key = ?")
            .bind(key)
            .execute(&pool)
            .await?;
    }
    println!(
        "pruned {} board(s) from data/config/workday_boards.yaml",
        removed.len()
    );
    Ok(())
}

fn write_tombstones(
    removed: &[String],
    reasons: &std::collections::BTreeMap<String, String>,
) -> Result<()> {
    let path = Path::new("data/config/workday_tombstones.yaml");
    let existing = fs::read_to_string(path).unwrap_or_default();
    let mut body = existing;
    if !body.contains("tombstones:") {
        body.push_str(
            "# Workday boards removed as structurally dead, by `jh prune`.\n\
             # Harvest skips these triples when it rebuilds the board file.\n\
             tombstones:\n",
        );
    }
    let stamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    for key in removed {
        if body.contains(&format!("key: {key}")) {
            continue;
        }
        let parts: Vec<&str> = key.split('|').collect();
        let reason = reasons
            .get(key)
            .map(String::as_str)
            .unwrap_or("structural failure");
        body.push_str(&format!(
            "  - {{key: {key}, tenant: {}, wd: {}, site: {}, pruned_at: {stamp}, reason: {}}}\n",
            parts.first().copied().unwrap_or(""),
            parts.get(1).copied().unwrap_or(""),
            parts.get(2).copied().unwrap_or(""),
            serde_json::to_string(reason)?
        ));
    }
    atomic_write(path, &body)
}

fn atomic_write(path: &Path, body: &str) -> Result<()> {
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    fs::write(&tmp, body)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

async fn run_probe(db: PathBuf, report: bool, min_jobs: usize, workers: usize) -> Result<()> {
    use futures::stream::{self, StreamExt};
    let pool = open_pool(&db)?;
    kangaroo_db::migrate(&pool).await?;
    let ctx = scan_context()?;
    let cfg: serde_yaml::Value =
        serde_yaml::from_str("companies_files: [data/config/ats_boards.yaml]")?;
    let existing: Vec<kangaroo_scan::sources::ats::Board> =
        kangaroo_scan::sources::ats::load_entries(&ctx, &cfg);
    let probe_kinds = ["greenhouse", "ashby", "lever", "workable"];
    let mut candidates: Vec<kangaroo_scan::sources::ats::Board> = existing
        .iter()
        .filter(|board| probe_kinds.contains(&board.ats.as_str()))
        .cloned()
        .collect();
    let existing_keys: std::collections::HashSet<String> = candidates
        .iter()
        .map(|board| format!("{}|{}", board.ats, board.slug))
        .collect();
    let mut seen = existing_keys.clone();
    let harvested = sqlx::query("SELECT payload_json FROM harvested_boards")
        .fetch_all(&pool)
        .await?;
    for row in harvested {
        let Ok(board) = serde_json::from_str::<serde_json::Value>(
            row.get::<String, _>("payload_json").as_str(),
        ) else {
            continue;
        };
        let (Some(ats), Some(slug)) = (
            board.get("ats").and_then(serde_json::Value::as_str),
            board.get("slug").and_then(serde_json::Value::as_str),
        ) else {
            continue;
        };
        if !probe_kinds.contains(&ats) || !seen.insert(format!("{ats}|{slug}")) {
            continue;
        }
        candidates.push(kangaroo_scan::sources::ats::Board {
            name: board
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(slug)
                .to_owned(),
            ats: ats.to_owned(),
            slug: slug.to_owned(),
            tenant: String::new(),
            wd: String::new(),
            site: String::new(),
        });
    }

    let results: Vec<(kangaroo_scan::sources::ats::Board, bool, usize)> =
        stream::iter(candidates.into_iter().map(|board| {
            let ctx = ctx.clone();
            async move {
                let jobs = probe_board(&ctx, &board).await;
                let ok = jobs >= min_jobs;
                (board, ok, jobs)
            }
        }))
        .buffer_unordered(workers.max(1))
        .collect()
        .await;

    let old: std::collections::HashMap<String, i64> =
        sqlx::query_as::<_, (String, i64)>("SELECT key, streak FROM probe_failures")
            .fetch_all(&pool)
            .await
            .unwrap_or_default()
            .into_iter()
            .collect();
    let stamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let mut kept = Vec::new();
    let mut graced = 0;
    for (board, ok, jobs) in &results {
        let key = format!("{}/{}", board.ats, board.slug);
        if *ok {
            let _ = sqlx::query("DELETE FROM probe_failures WHERE key = ?")
                .bind(&key)
                .execute(&pool)
                .await;
            kept.push((board.clone(), *jobs));
            continue;
        }
        let streak = old.get(&key).copied().unwrap_or(0) + 1;
        let _ = sqlx::query(
            "INSERT INTO probe_failures(key,streak,last_error,updated_at) VALUES(?,?,?,?)
             ON CONFLICT(key) DO UPDATE SET streak=excluded.streak, updated_at=excluded.updated_at",
        )
        .bind(&key)
        .bind(streak)
        .bind("probe did not answer")
        .bind(&stamp)
        .execute(&pool)
        .await;
        if streak == 1 && existing_keys.contains(&format!("{}|{}", board.ats, board.slug)) {
            graced += 1;
            kept.push((board.clone(), *jobs));
        }
    }
    kept.sort_by(|a, b| {
        (a.0.ats.as_str(), usize::MAX - a.1, a.0.name.to_lowercase()).cmp(&(
            b.0.ats.as_str(),
            usize::MAX - b.1,
            b.0.name.to_lowercase(),
        ))
    });
    let dead = results.len() - kept.len();
    eprintln!(
        "[probe] {} candidates, {} live boards, {dead} dead ({graced} held on a first failure)",
        results.len(),
        kept.len()
    );
    if report {
        return Ok(());
    }
    let mut by_ats: std::collections::BTreeMap<
        &str,
        Vec<&(kangaroo_scan::sources::ats::Board, usize)>,
    > = std::collections::BTreeMap::new();
    for row in &kept {
        by_ats.entry(row.0.ats.as_str()).or_default().push(row);
    }
    let mut body = String::from(
        "# Public ATS job boards, verified by `jh probe-ats`.\n\
         #\n\
         # Every entry answered its board API with at least one open posting when it was last\n\
         # probed -- except entries held on a first failure, which answered on the pass before.\n\
         # A board is dropped only after two consecutive failed probes.\n",
    );
    for (ats, rows) in by_ats {
        body.push_str(&format!("\n# --- {ats} ({} live boards) ---\n", rows.len()));
        body.push_str(&format!("boards_{ats}:\n"));
        for (board, _) in rows {
            body.push_str(&format!(
                "  - {{name: {}, ats: {ats}, slug: {}}}\n",
                serde_json::to_string(&board.name)?,
                serde_json::to_string(&board.slug)?
            ));
        }
    }
    atomic_write(Path::new("data/config/ats_boards.yaml"), &body)?;
    eprintln!(
        "[probe] wrote data/config/ats_boards.yaml ({} boards)",
        kept.len()
    );
    Ok(())
}

async fn run_harvest(db: PathBuf, kind: HarvestKind, report: bool) -> Result<()> {
    let _ = open_pool(&db)?;
    let ctx = scan_context()?;
    let sources = read_yaml(Path::new("data/config/sources.yaml"))?;
    let feed_url = sources
        .get("sources")
        .and_then(serde_yaml::Value::as_sequence)
        .and_then(|sources| {
            sources.iter().find(|source| {
                source.get("id").and_then(serde_yaml::Value::as_str) == Some("newgrad_json")
            })
        })
        .and_then(|source| source.get("url"))
        .and_then(serde_yaml::Value::as_str)
        .context("sources.yaml has no newgrad_json url")?;
    let raw = ctx
        .http
        .get_text_with_timeout(feed_url, &[], std::time::Duration::from_secs(90), 1)
        .await
        .context("could not fetch the Simplify feed")?;
    let data: serde_json::Value = serde_json::from_str(&raw).context("feed was not JSON")?;
    let records: Vec<serde_json::Value> = data
        .as_array()
        .cloned()
        .or_else(|| {
            data.get("listings")
                .and_then(serde_json::Value::as_array)
                .cloned()
        })
        .unwrap_or_default();
    match kind {
        HarvestKind::Workday => harvest_workday(&records, report),
        HarvestKind::Jsonboards => harvest_jsonboards(&records, report),
    }
}

async fn run_migrate_locations(db: PathBuf, dry_run: bool) -> Result<()> {
    if let Some(parent) = db.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let pool = kangaroo_db::connect(db.to_str().context("database path is not UTF-8")?).await?;
    kangaroo_db::migrate(&pool).await?;
    let filters = read_yaml(Path::new("data/config/filters.yaml"))?;
    let tiers = kangaroo_core::filters::tier_lookup(&filters);

    let rows = sqlx::query("SELECT id, location, tier, requires_us_work_auth FROM postings")
        .fetch_all(&pool)
        .await?;
    let mut tier_changes =
        std::collections::BTreeMap::<(Option<String>, Option<String>), usize>::new();
    let mut flag_changes = 0usize;
    for row in &rows {
        let id: String = row.get("id");
        let location: Option<String> = row.get("location");
        let old_tier: Option<String> = row.get("tier");
        let old_flag: i64 = row.get("requires_us_work_auth");

        let (ok, _kind, tier) =
            kangaroo_core::filters::location_matches(location.as_deref(), &filters);
        let tier = ok.then_some(tier).flatten();
        let country = tier
            .as_deref()
            .and_then(|name| tiers.iter().find(|t| t.0.as_str() == name))
            .map(|t| t.2.as_str())
            .unwrap_or("");
        let (ca, us) = kangaroo_core::filters::region_signals(location.as_deref().unwrap_or(""));
        let flag = country == "usa" || (us && !ca);

        if tier != old_tier {
            *tier_changes
                .entry((old_tier.clone(), tier.clone()))
                .or_default() += 1;
        }
        if flag != (old_flag != 0) {
            flag_changes += 1;
        }
        if !dry_run {
            sqlx::query("UPDATE postings SET tier = ?, requires_us_work_auth = ? WHERE id = ?")
                .bind(&tier)
                .bind(flag as i64)
                .bind(&id)
                .execute(&pool)
                .await?;
        }
    }

    let applied = sqlx::query("SELECT posting_id, snapshot_json FROM applied")
        .fetch_all(&pool)
        .await?;
    let mut applied_changed = 0usize;
    for row in &applied {
        let posting_id: String = row.get("posting_id");
        let raw: String = row.get("snapshot_json");
        let Ok(mut snapshot) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        let location = snapshot
            .get("location")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let old_tier = snapshot
            .get("tier")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let (ok, _kind, tier) =
            kangaroo_core::filters::location_matches(location.as_deref(), &filters);
        let tier = ok.then_some(tier).flatten();
        if tier != old_tier {
            applied_changed += 1;
        }
        if !dry_run && let Some(object) = snapshot.as_object_mut() {
            match &tier {
                Some(t) => {
                    object.insert("tier".into(), Value::String(t.clone()));
                }
                None => {
                    object.remove("tier");
                }
            }
            sqlx::query("UPDATE applied SET snapshot_json = ? WHERE posting_id = ?")
                .bind(snapshot.to_string())
                .bind(&posting_id)
                .execute(&pool)
                .await?;
        }
    }

    println!(
        "migrate-locations: {} postings, {} applied row(s) changed",
        rows.len(),
        applied_changed
    );
    for ((old, new), count) in &tier_changes {
        println!(
            "  {count:4}  {} -> {}",
            old.as_deref().unwrap_or("(none)"),
            new.as_deref().unwrap_or("(none)")
        );
    }
    if flag_changes > 0 {
        println!("  {flag_changes} flag change(s)");
    }
    if dry_run {
        println!("dry run: nothing written");
    }
    Ok(())
}

fn harvest_workday(records: &[serde_json::Value], report: bool) -> Result<()> {
    let patterns = [
        regex::Regex::new(
            r"(?i)https?://([a-z0-9-]+)\.(wd\d+)\.myworkdayjobs\.com/(?:en-US/)?([^/]+)/job/",
        )?,
        regex::Regex::new(
            r"(?i)https?://(wd\d+)\.myworkdaysite\.com/(?:en-US/)?recruiting/([^/]+)/([^/]+)/job/",
        )?,
    ];
    let mut counts: std::collections::BTreeMap<(String, String, String), usize> =
        std::collections::BTreeMap::new();
    for record in records {
        let url = record
            .get("url")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        if let Some(hit) = patterns[0].captures(url) {
            let key = (
                hit[1].to_lowercase(),
                hit[2].to_lowercase(),
                hit[3].to_owned(),
            );
            *counts.entry(key).or_default() += 1;
        } else if let Some(hit) = patterns[1].captures(url) {
            let key = (
                hit[2].to_lowercase(),
                hit[1].to_lowercase(),
                hit[3].to_owned(),
            );
            *counts.entry(key).or_default() += 1;
        }
    }
    let buried = if let Ok(text) = fs::read_to_string("data/config/workday_tombstones.yaml") {
        if let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(&text) {
            value
                .get("tombstones")
                .and_then(serde_yaml::Value::as_sequence)
                .into_iter()
                .flatten()
                .filter_map(|row| row.get("key").and_then(serde_yaml::Value::as_str))
                .map(|key| key.to_lowercase())
                .collect::<std::collections::HashSet<_>>()
        } else {
            Default::default()
        }
    } else {
        Default::default()
    };
    let mut rows: Vec<((String, String, String), usize)> = counts
        .into_iter()
        .filter(|((tenant, wd, site), _)| {
            !buried.contains(&format!("{tenant}|{wd}|{site}").to_lowercase())
        })
        .collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    eprintln!(
        "[harvest] feed has {} records -> {} Workday boards",
        records.len(),
        rows.len()
    );
    if report {
        return Ok(());
    }
    let mut body = String::from(
        "# Workday boards, harvested from the Simplify feed.\n\
         #\n\
         # The tenant + wd-number + site triple comes out of the feed's own URLs.\n\
         # Regenerate with `jh harvest workday`. Pruned boards are in\n\
         # workday_tombstones.yaml and are skipped here.\n",
    );
    body.push_str("\nboards_workday:\n");
    for ((tenant, wd, site), urls) in rows {
        body.push_str(&format!(
            "  - {{name: {}, ats: workday, tenant: {}, wd: {}, site: {}, urls: {}}}\n",
            serde_json::to_string(&tenant)?,
            tenant,
            wd,
            site,
            urls
        ));
    }
    atomic_write(Path::new("data/config/workday_boards.yaml"), &body)?;
    eprintln!("[harvest] wrote data/config/workday_boards.yaml");
    Ok(())
}

fn harvest_jsonboards(records: &[serde_json::Value], report: bool) -> Result<()> {
    let vendors: [(&str, &str); 7] = [
        ("workable", r#"(?i)apply\.workable\.com/([^/"'?#]+)/j/"#),
        (
            "smartrecruiters",
            r#"(?i)jobs\.smartrecruiters\.com/([^/"'?#]+)/"#,
        ),
        ("rippling", r#"(?i)ats\.rippling\.com/([^/"'?#]+)/jobs/"#),
        ("bamboohr", r"(?i)https?://([a-z0-9-]+)\.bamboohr\.com/"),
        ("recruitee", r"(?i)https?://([a-z0-9-]+)\.recruitee\.com/"),
        ("breezy", r"(?i)https?://([a-z0-9-]+)\.breezy\.hr/"),
        ("teamtailor", r"(?i)https?://([a-z0-9-]+)\.teamtailor\.com/"),
    ];
    let mut found: std::collections::BTreeMap<&str, Vec<String>> =
        std::collections::BTreeMap::new();
    for (vendor, pattern) in vendors {
        let regex = regex::Regex::new(pattern)?;
        let mut seen = std::collections::HashSet::new();
        for record in records {
            let url = record
                .get("url")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            if let Some(hit) = regex.captures(url) {
                let slug = hit[1].to_owned();
                if seen.insert(slug.to_lowercase()) {
                    found.entry(vendor).or_default().push(slug);
                }
            }
        }
        if let Some(slugs) = found.get_mut(vendor) {
            slugs.sort_by_key(|slug| slug.to_lowercase());
        }
    }
    let total: usize = found.values().map(Vec::len).sum();
    eprintln!(
        "[harvest] feed has {} records -> {} JSON-vendor boards",
        records.len(),
        total
    );
    for (vendor, slugs) in &found {
        eprintln!("   {vendor:16} {:4}", slugs.len());
    }
    if report {
        return Ok(());
    }
    let mut body = String::from(
        "# JSON-vendor job boards, harvested from the Simplify feed.\n\
         # Regenerate with `jh harvest jsonboards`.\n",
    );
    for (vendor, slugs) in found {
        body.push_str(&format!(
            "\n# --- {vendor} ({} boards) ---\nboards_{vendor}:\n",
            slugs.len()
        ));
        for slug in slugs {
            body.push_str(&format!(
                "  - {{name: {}, ats: {vendor}, slug: {}}}\n",
                serde_json::to_string(&slug)?,
                serde_json::to_string(&slug)?
            ));
        }
    }
    atomic_write(Path::new("data/config/json_boards.yaml"), &body)?;
    eprintln!("[harvest] wrote data/config/json_boards.yaml");
    Ok(())
}

async fn probe_board(
    ctx: &kangaroo_scan::ScanContext,
    board: &kangaroo_scan::sources::ats::Board,
) -> usize {
    let url = match board.ats.as_str() {
        "greenhouse" => format!(
            "https://boards-api.greenhouse.io/v1/boards/{}/jobs",
            board.slug
        ),
        "ashby" => format!(
            "https://api.ashbyhq.com/posting-api/job-board/{}",
            board.slug
        ),
        "lever" => format!("https://api.lever.co/v0/postings/{}?mode=json", board.slug),
        "workable" => format!(
            "https://apply.workable.com/api/v1/widget/accounts/{}?details=true",
            board.slug
        ),
        _ => return 0,
    };
    let Some(text) = ctx
        .http
        .get_text_with_timeout(
            &url,
            &[("Accept", "application/json")],
            std::time::Duration::from_secs(20),
            0,
        )
        .await
    else {
        return 0;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return 0;
    };
    match value {
        serde_json::Value::Array(items) => items.len(),
        other => ["jobs", "results", "content"]
            .iter()
            .find_map(|key| other.get(key).and_then(serde_json::Value::as_array))
            .map(Vec::len)
            .unwrap_or(0),
    }
}

fn read_json(path: &Path) -> Option<Value> {
    fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
}
fn lines(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}
fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_owned)
}
fn num(v: &Value, k: &str) -> Option<i64> {
    v.get(k).and_then(Value::as_i64)
}
async fn import_all(pool: &SqlitePool, dir: &Path) -> Result<()> {
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for table in [
        "postings",
        "scan_runs",
        "source_runs",
        "descriptions",
        "seen",
        "board_health",
        "cache",
        "harvested_boards",
        "probe_failures",
        "applied",
        "briefs",
        "brief_artifacts",
    ] {
        let _ = counts.insert(table.to_owned(), 0);
    }
    let posts = lines(&dir.join("portal_feed.jsonl"));
    if !posts.is_empty() {
        let mut tx = pool.begin().await?;
        for p in posts {
            let id = s(&p, "id").unwrap_or_default();
            if id.is_empty() {
                continue;
            }
            let r=sqlx::query("INSERT OR IGNORE INTO postings(id,source,ats,raw_id,company,title,location,url,salary_raw,posted,age_days,scanned_at,tier,score,level_ii,requires_us_work_auth,first_seen_at,last_seen_at,expires_on,retired_at,keywords_json,skills_have_json,skills_gap_json,emails_json,verified_emails_json,has_text,below_comp_floor,salary_annual_min,salary_annual_max,salary_estimated,posted_derived) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)")
 .bind(id).bind(s(&p,"source").unwrap_or_default()).bind(s(&p,"ats")).bind(s(&p,"raw_id")).bind(s(&p,"company").unwrap_or_default()).bind(s(&p,"title").unwrap_or_default()).bind(s(&p,"location")).bind(s(&p,"url")).bind(s(&p,"salary")).bind(s(&p,"posted")).bind(num(&p,"age_days")).bind(s(&p,"scanned_at")).bind(s(&p,"tier")).bind(num(&p,"score")).bind(p.get("level_ii").and_then(Value::as_bool).unwrap_or(false) as i64).bind(p.get("requires_us_work_auth").and_then(Value::as_bool).unwrap_or(false) as i64).bind(s(&p,"first_seen_at").unwrap_or_else(||s(&p,"scanned_at").unwrap_or_default())).bind(s(&p,"last_seen_at").unwrap_or_else(||s(&p,"scanned_at").unwrap_or_default())).bind(s(&p,"expires_on")).bind(s(&p,"retired_at")).bind(p.get("keywords").cloned().unwrap_or(json!([])).to_string()).bind(p.get("skills_have").cloned().unwrap_or(json!([])).to_string()).bind(p.get("skills_gap").cloned().unwrap_or(json!([])).to_string()).bind(p.get("emails").cloned().unwrap_or(json!([])).to_string()).bind(p.get("verified_emails").cloned().unwrap_or(json!([])).to_string()).bind(p.get("has_text").and_then(Value::as_bool).unwrap_or(false) as i64).bind(p.get("below_comp_floor").and_then(Value::as_bool).unwrap_or(false) as i64).bind(num(&p,"salary_annual_min")).bind(num(&p,"salary_annual_max")).bind(p.get("salary_estimated").and_then(Value::as_bool).unwrap_or(false) as i64).bind(p.get("posted_derived").and_then(Value::as_bool).unwrap_or(false) as i64).execute(&mut *tx).await?;
            counts
                .entry("postings".into())
                .and_modify(|n| *n += r.rows_affected() as usize)
                .or_insert(r.rows_affected() as usize);
        }
        tx.commit().await?;
    }
    import_seen(pool, dir, &mut counts).await?;
    import_applied(pool, dir, &mut counts).await?;
    import_misc(pool, dir, &mut counts).await?;
    for (t, n) in counts {
        println!("{t}: {n}");
    }
    Ok(())
}
async fn import_seen(
    pool: &SqlitePool,
    dir: &Path,
    c: &mut std::collections::BTreeMap<String, usize>,
) -> Result<()> {
    let rows = lines(&dir.join("seen.jsonl"));
    if rows.is_empty() {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    for v in rows {
        let id = s(&v, "id").unwrap_or_default();
        let r = sqlx::query(
            "INSERT OR IGNORE INTO seen(id,reported,first_seen_at,last_seen_at) VALUES(?,?,?,?)",
        )
        .bind(id)
        .bind(v.get("reported").and_then(Value::as_bool).unwrap_or(false) as i64)
        .bind(
            s(&v, "first_seen_at")
                .or_else(|| s(&v, "first_seen"))
                .unwrap_or_default(),
        )
        .bind(
            s(&v, "last_seen_at")
                .or_else(|| s(&v, "last_seen"))
                .unwrap_or_default(),
        )
        .execute(&mut *tx)
        .await?;
        *c.entry("seen".into()).or_default() += r.rows_affected() as usize;
    }
    tx.commit().await?;
    Ok(())
}
async fn import_applied(
    pool: &SqlitePool,
    dir: &Path,
    c: &mut std::collections::BTreeMap<String, usize>,
) -> Result<()> {
    let Some(v) = read_json(&dir.join("applied.json")) else {
        return Ok(());
    };
    let rows: Vec<(String, Value)> = if let Some(rows) = v
        .as_array()
        .or_else(|| v.get("applied").and_then(Value::as_array))
    {
        rows.iter()
            .map(|a| {
                (
                    s(a, "id")
                        .or_else(|| s(a, "posting_id"))
                        .unwrap_or_default(),
                    a.clone(),
                )
            })
            .collect()
    } else if let Some(rows) = v.as_object() {
        rows.iter().map(|(id, a)| (id.clone(), a.clone())).collect()
    } else {
        Vec::new()
    };
    if rows.is_empty() {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    for (id, a) in rows {
        let r = sqlx::query(
            "INSERT OR IGNORE INTO applied(posting_id,applied_at,snapshot_json) VALUES(?,?,?)",
        )
        .bind(id)
        .bind(s(&a, "applied_at").unwrap_or_default())
        .bind(a.to_string())
        .execute(&mut *tx)
        .await?;
        *c.entry("applied".into()).or_default() += r.rows_affected() as usize;
    }
    tx.commit().await?;
    Ok(())
}
async fn import_misc(
    pool: &SqlitePool,
    dir: &Path,
    c: &mut std::collections::BTreeMap<String, usize>,
) -> Result<()> {
    if let Some(d) = read_json(&dir.join("descriptions.json"))
        && let Some(m) = d.as_object()
    {
        let mut tx = pool.begin().await?;
        for (id, text) in m {
            if let Some(t) = text.as_str() {
                let r =
                    sqlx::query("INSERT OR IGNORE INTO descriptions(posting_id,text) VALUES(?,?)")
                        .bind(id)
                        .bind(t)
                        .execute(&mut *tx)
                        .await?;
                *c.entry("descriptions".into()).or_default() += r.rows_affected() as usize;
            }
        }
        tx.commit().await?
    }
    if let Some(data) = read_json(&dir.join("board_health.json")) {
        let boards = data.get("boards").and_then(Value::as_object);
        if let Some(boards) = boards {
            let mut tx = pool.begin().await?;
            for (key, b) in boards {
                let r=sqlx::query("INSERT OR IGNORE INTO board_health(key,vendor,slug,tenant,wd,site,status,consecutive_failures,last_ok_at,last_checked_at,next_check_at,last_error,error_kind,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)")
                    .bind(key).bind(s(b,"vendor")).bind(s(b,"slug")).bind(s(b,"tenant")).bind(s(b,"wd")).bind(s(b,"site")).bind(s(b,"status")).bind(num(b,"consecutive_failures").unwrap_or(0)).bind(s(b,"last_ok_at")).bind(s(b,"last_checked_at")).bind(s(b,"next_check_at")).bind(s(b,"last_error")).bind(s(b,"error_kind")).bind(s(b,"updated_at").or_else(||s(&data,"updated_at")).unwrap_or_default()).execute(&mut *tx).await?;
                *c.entry("board_health".into()).or_default() += r.rows_affected() as usize;
            }
            tx.commit().await?;
        }
    }
    if let Some(data) = read_json(&dir.join("workday_sweep.json")) {
        let updated = s(&data, "swept_at")
            .or_else(|| s(&data, "updated_at"))
            .unwrap_or_default();
        let r = sqlx::query(
            "INSERT OR IGNORE INTO cache(key,value_json,updated_at) VALUES('workday_sweep',?,?)",
        )
        .bind(data.to_string())
        .bind(updated)
        .execute(pool)
        .await?;
        *c.entry("cache".into()).or_default() += r.rows_affected() as usize;
    }
    if let Some(data) = read_json(&dir.join("sg_boards.json"))
        && let Some(boards) = data.get("boards").and_then(Value::as_array)
    {
        let mut tx = pool.begin().await?;
        for b in boards {
            let key = format!(
                "{}|{}",
                s(b, "ats").unwrap_or_default(),
                s(b, "slug").unwrap_or_default()
            );
            let r=sqlx::query("INSERT OR IGNORE INTO harvested_boards(key,source,payload_json,first_seen_at,last_seen_at) VALUES(?,?,?,?,?)").bind(key).bind("startups_gallery").bind(b.to_string()).bind(s(b,"first_seen").or_else(||s(&data,"updated_at")).unwrap_or_default()).bind(s(b,"last_seen").or_else(||s(&data,"updated_at")).unwrap_or_default()).execute(&mut *tx).await?;
            *c.entry("harvested_boards".into()).or_default() += r.rows_affected() as usize;
        }
        tx.commit().await?
    }
    if let Some(data) = read_json(&dir.join("ats_probe_failures.json"))
        && let Some(rows) = data.get("failures").and_then(Value::as_object)
    {
        let mut tx = pool.begin().await?;
        for (key, v) in rows {
            let streak = v.get("failures").and_then(Value::as_i64).unwrap_or(0);
            let r=sqlx::query("INSERT OR IGNORE INTO probe_failures(key,streak,last_error,updated_at) VALUES(?,?,?,?)").bind(key).bind(streak).bind(s(v,"last_error")).bind(s(&data,"updated_at").unwrap_or_default()).execute(&mut *tx).await?;
            *c.entry("probe_failures".into()).or_default() += r.rows_affected() as usize;
        }
        tx.commit().await?
    }
    if let Some(expired) = read_json(&dir.join("portal_expired.json"))
        && let Some(rows) = expired.as_object()
    {
        let mut tx = pool.begin().await?;
        for (id, v) in rows {
            let expires_on = s(v, "expires_on");
            sqlx::query("UPDATE postings SET retired_at=COALESCE(retired_at,?), expires_on=COALESCE(expires_on,?) WHERE id=? AND (retired_at IS NULL OR (expires_on IS NULL AND ? IS NOT NULL))").bind(s(v,"retired_at").or_else(||expires_on.clone()).unwrap_or_default()).bind(expires_on.clone()).bind(id).bind(expires_on).execute(&mut *tx).await?;
        }
        tx.commit().await?
    }
    let brief_dir = dir.join("briefs");
    if let Ok(entries) = fs::read_dir(&brief_dir) {
        let mut tx = pool.begin().await?;
        let mut paths: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
        paths.sort_by_key(|path| {
            path.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                n.ends_with(".raw.json")
                    || n.ends_with(".agent.json")
                    || n.ends_with(".review.json")
            })
        });
        for path in paths {
            let Some(name) = path.file_name().and_then(|v| v.to_str()) else {
                continue;
            };
            let Some(raw) = read_json(&path) else {
                continue;
            };
            let (id, kind) = if let Some(id) = name.strip_suffix(".raw.json") {
                (id, Some("raw"))
            } else if let Some(id) = name.strip_suffix(".agent.json") {
                (id, Some("agent"))
            } else if let Some(id) = name.strip_suffix(".review.json") {
                (id, Some("review"))
            } else if let Some(id) = name.strip_suffix(".json") {
                (id, None)
            } else {
                continue;
            };
            if let Some(kind) = kind {
                let r=sqlx::query("INSERT OR IGNORE INTO brief_artifacts(posting_id,kind,payload_json,created_at) VALUES(?,?,?,?)").bind(id).bind(kind).bind(raw.to_string()).bind(s(&raw,"generated_at").unwrap_or_default()).execute(&mut *tx).await?;
                *c.entry("brief_artifacts".into()).or_default() += r.rows_affected() as usize;
            } else {
                let posting_id = s(&raw, "posting_id").unwrap_or_else(|| id.to_owned());
                let r=sqlx::query("INSERT OR IGNORE INTO briefs(posting_id,status,engine,generated_at,started_at,payload_json,error) VALUES(?,?,?,?,?,?,?)").bind(posting_id).bind(s(&raw,"status").unwrap_or_else(||"unknown".into())).bind(s(&raw,"engine")).bind(s(&raw,"generated_at")).bind(s(&raw,"started_at")).bind(raw.to_string()).bind(s(&raw,"error")).execute(&mut *tx).await?;
                *c.entry("briefs".into()).or_default() += r.rows_affected() as usize;
            }
        }
        tx.commit().await?;
    }
    if let Some(scan) = read_json(&dir.join("last_scan.json")) {
        let started = s(&scan, "ran_at").unwrap_or_default();
        if !started.is_empty() {
            if let Some(matched) = scan.get("matched").and_then(Value::as_array) {
                let mut tx = pool.begin().await?;
                for p in matched {
                    let id = s(p, "id").unwrap_or_default();
                    if id.is_empty() {
                        continue;
                    }
                    let r=sqlx::query("INSERT OR IGNORE INTO postings(id,source,raw_id,company,title,location,url,salary_raw,posted,age_days,scanned_at,tier,score,first_seen_at,last_seen_at,expires_on,retired_at,keywords_json,emails_json,verified_emails_json) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)")
                        .bind(id).bind(s(p,"source").unwrap_or_default()).bind(s(p,"raw_id")).bind(s(p,"company").unwrap_or_default()).bind(s(p,"title").unwrap_or_default()).bind(s(p,"location")).bind(s(p,"url")).bind(s(p,"salary")).bind(s(p,"posted")).bind(num(p,"age_days")).bind(started.clone()).bind(s(p,"tier")).bind(num(p,"score")).bind(s(p,"first_seen_at").or_else(||s(p,"first_seen")).unwrap_or_else(||started.clone())).bind(s(p,"last_seen_at").or_else(||s(p,"last_seen")).unwrap_or_else(||started.clone())).bind(s(p,"expires_on")).bind(started.clone()).bind(p.get("keywords").cloned().unwrap_or(json!([])).to_string()).bind(p.get("emails").cloned().unwrap_or(json!([])).to_string()).bind(p.get("verified_emails").cloned().unwrap_or(json!([])).to_string()).execute(&mut *tx).await?;
                    *c.entry("postings".into()).or_default() += r.rows_affected() as usize;
                }
                tx.commit().await?;
            }
            let exists: i64 = sqlx::query("SELECT count(*) AS n FROM scan_runs WHERE started_at=?")
                .bind(&started)
                .fetch_one(pool)
                .await?
                .get("n");
            if exists == 0 {
                let mut tx = pool.begin().await?;
                let r=sqlx::query("INSERT INTO scan_runs(mode,status,started_at,finished_at,matched,stats_json) VALUES(?,?,?,?,?,?)").bind("full").bind("ok").bind(started).bind(s(&scan,"finished_at")).bind(scan.get("matched").and_then(Value::as_array).map(|a|a.len() as i64)).bind(scan.to_string()).execute(&mut *tx).await?;
                let run_id = r.last_insert_rowid();
                *c.entry("scan_runs".into()).or_default() += r.rows_affected() as usize;
                let found = scan.get("sources").and_then(Value::as_object);
                let seconds = scan.get("source_seconds").and_then(Value::as_object);
                if let Some(found) = found {
                    for (source, n) in found {
                        let r=sqlx::query("INSERT OR IGNORE INTO source_runs(scan_run_id,source,found,kept,seconds) VALUES(?,?,?,?,?)").bind(run_id).bind(source).bind(n.as_i64().unwrap_or(0)).bind(0_i64).bind(seconds.and_then(|m|m.get(source)).and_then(Value::as_f64).unwrap_or(0.0)).execute(&mut *tx).await?;
                        *c.entry("source_runs".into()).or_default() += r.rows_affected() as usize;
                    }
                }
                tx.commit().await?;
            }
        }
    }
    Ok(())
}
