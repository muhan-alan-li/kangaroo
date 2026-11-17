//! One scan run: read the source registry, let the sources work concurrently,
//! collapse and filter, then write the postings, the ledger and the run row.

use crate::EngineEvent;
use crate::boardstate;
use chrono::{Duration, NaiveDate, SecondsFormat, Utc};
use kangaroo_core::pipeline::{normalize_and_filter, sort_key, tier_counts};
use kangaroo_core::posting::collapse_requisitions;
use kangaroo_scan::ScanContext;
use kangaroo_scan::sources::{self, Source};
use serde_json::{Value, json};
use serde_yaml::Value as Yaml;
use sqlx::{Row, SqlitePool};
use std::{collections::BTreeMap, path::Path, time::Instant};
use tokio::sync::watch;

#[derive(Debug, Default, serde::Serialize)]
pub struct ScanOutcome {
    pub matched: usize,
    pub collected: usize,
    pub per_source: BTreeMap<String, usize>,
    pub source_seconds: BTreeMap<String, f64>,
    pub dropped: BTreeMap<String, usize>,
    pub tier_counts: BTreeMap<String, usize>,
}

struct Found {
    id: String,
    postings: Vec<Value>,
    seconds: f64,
    error: Option<String>,
}

async fn run_source(
    id: String,
    cfg: Yaml,
    source: Box<dyn Source>,
    ctx: ScanContext,
    filters: Yaml,
) -> Found {
    let started = Instant::now();
    match source.fetch(&ctx, &cfg, &filters).await {
        Ok(postings) => Found {
            id,
            postings,
            seconds: started.elapsed().as_secs_f64(),
            error: None,
        },
        Err(error) => {
            eprintln!("[scan] source {id} raised: {error:#}");
            Found {
                id,
                postings: Vec::new(),
                seconds: started.elapsed().as_secs_f64(),
                error: Some(format!("{error:#}")),
            }
        }
    }
}

pub async fn run(
    pool: &SqlitePool,
    mode: &str,
    events: &flume::Sender<EngineEvent>,
    cancel: &watch::Receiver<bool>,
    only: &[String],
) -> anyhow::Result<ScanOutcome> {
    let root = std::env::var_os("KANGAROO_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| ".".into()));
    let config = root.join("data").join("config");
    let filters = read_yaml(&config.join("filters.yaml")).unwrap_or(Yaml::Null);
    let sources_doc = read_yaml(&config.join("sources.yaml")).unwrap_or(Yaml::Null);
    let http = kangaroo_scan::HttpClient::from_sources_file(&config.join("sources.yaml"))?;
    let mut ctx = ScanContext::with_root(http, &root);
    let max_cache_hours = sources_doc
        .get("sources")
        .and_then(Yaml::as_sequence)
        .into_iter()
        .flatten()
        .find(|cfg| cfg.get("kind").and_then(Yaml::as_str) == Some("ats"))
        .and_then(|cfg| cfg.get("workday"))
        .and_then(|workday| workday.get("max_cache_age_hours"))
        .and_then(Yaml::as_f64)
        .unwrap_or(30.0);
    if let Ok(Some(cached)) = boardstate::read_sweep(pool, max_cache_hours).await {
        ctx.workday_cache = Some(std::sync::Arc::new(cached));
    }
    let started = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);

    let run_id: i64 = sqlx::query(
        "INSERT INTO scan_runs(mode,status,started_at,matched,stats_json) VALUES(?, 'running', ?, 0, '{}')",
    )
    .bind(mode)
    .bind(&started)
    .execute(pool)
    .await?
    .last_insert_rowid();

    let mut jobs: Vec<(String, Yaml, Box<dyn Source>)> = Vec::new();
    for cfg in sources_doc
        .get("sources")
        .and_then(Yaml::as_sequence)
        .into_iter()
        .flatten()
    {
        if cfg.get("enabled").and_then(Yaml::as_bool) != Some(true) {
            continue;
        }
        let Some(id) = cfg.get("id").and_then(Yaml::as_str) else {
            continue;
        };
        if !only.is_empty() && !only.iter().any(|wanted| wanted == id) {
            continue;
        }
        let Some(kind) = cfg.get("kind").and_then(Yaml::as_str) else {
            continue;
        };
        match sources::build(kind) {
            Some(source) => jobs.push((id.to_owned(), cfg.clone(), source)),
            None => eprintln!("[scan] unknown source kind {kind:?}"),
        }
    }

    let mut set = tokio::task::JoinSet::new();
    for (id, cfg, source) in jobs {
        let ctx = ctx.clone();
        let filters = filters.clone();
        set.spawn(async move { run_source(id, cfg, source, ctx, filters).await });
    }
    let mut results: Vec<Found> = Vec::new();
    while let Some(result) = set.join_next().await {
        match result {
            Ok(found) => results.push(found),
            Err(error) => eprintln!("[scan] source task failed: {error}"),
        }
    }

    let mut outcome = ScanOutcome::default();
    for found in &results {
        *outcome.per_source.entry(found.id.clone()).or_default() += found.postings.len();
        outcome
            .source_seconds
            .insert(found.id.clone(), (found.seconds * 100.0).round() / 100.0);
        sqlx::query(
            "INSERT INTO source_runs(scan_run_id,source,found,kept,seconds,error) VALUES(?,?,?,0,?,?)",
        )
        .bind(run_id)
        .bind(&found.id)
        .bind(found.postings.len() as i64)
        .bind(found.seconds)
        .bind(found.error.as_deref())
        .execute(pool)
        .await?;
    }
    let mut collected: Vec<Value> = Vec::new();
    for found in results {
        for mut posting in found.postings {
            posting["_scan_source"] = Value::String(found.id.clone());
            collected.push(posting);
        }
    }
    outcome.collected = collected.len();

    let notes = ctx.take_boards();
    let harvested = ctx.take_harvested();
    if !harvested.is_empty() {
        let stamp = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
        for board in harvested {
            let (Some(ats), Some(slug)) = (
                board.get("ats").and_then(Value::as_str),
                board.get("slug").and_then(Value::as_str),
            ) else {
                continue;
            };
            let _ = sqlx::query(
                "INSERT INTO harvested_boards(key,source,payload_json,first_seen_at,last_seen_at)
                 VALUES(?, 'startups_gallery', ?, ?, ?)
                 ON CONFLICT(key) DO UPDATE SET last_seen_at=excluded.last_seen_at,
                    payload_json=excluded.payload_json",
            )
            .bind(format!("{ats}|{slug}"))
            .bind(board.to_string())
            .bind(&stamp)
            .bind(&stamp)
            .execute(pool)
            .await;
        }
    }
    if !notes.is_empty() {
        let (trusted, reason) = boardstate::sweep_verdict(&notes, 0);
        if trusted {
            boardstate::write_notes(pool, &notes).await?;
            eprintln!("[scan] {}", boardstate::line(pool).await?);
        } else {
            eprintln!("[scan] not recording this run's board state: {reason}");
        }
    }
    ctx.http.guard.log_line_if_used();
    report_unprobed(pool, &root).await;

    if *cancel.borrow() {
        finish_run(pool, run_id, "cancelled", 0, &json!({ "cancelled": true })).await?;
        return Ok(outcome);
    }

    let (kept, dropped) = normalize_and_filter(collapse_requisitions(collected), &filters);
    outcome.dropped = dropped;
    outcome.matched = kept.len();
    outcome.tier_counts = tier_counts(&kept, &filters);

    let now = Utc::now();
    let now_text = now.to_rfc3339_opts(SecondsFormat::Secs, true);
    let today = now.date_naive();
    let window = filters
        .get("max_age_days")
        .and_then(Yaml::as_i64)
        .unwrap_or(7);
    let mut kept_per_source: BTreeMap<String, i64> = BTreeMap::new();

    // Sort before writing so the database's insertion order matches the page's
    // ordering for a first paint.
    let mut kept = kept;
    kept.sort_by_key(sort_key);
    crate::descriptions::harvest(pool, &ctx, &mut kept, crate::descriptions::DESC_BUDGET).await?;
    let claimed = kangaroo_core::keywords::profile_skills(&root);
    for posting in &mut kept {
        let text = posting
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let title = posting
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let company = posting
            .get("company")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let (keywords, have, gap) = kangaroo_core::keywords::extract_keywords(
            &format!("{title} {company} {text}"),
            &claimed,
            24,
        );
        let emails = kangaroo_core::keywords::extract_emails(&text);
        if let Some(object) = posting.as_object_mut() {
            object.insert("keywords".into(), json!(keywords));
            object.insert("skills_have".into(), json!(have));
            object.insert("skills_gap".into(), json!(gap));
            object.insert("emails".into(), json!(emails));
            object.insert("verified_emails".into(), json!([]));
        }
    }
    let mut tx = pool.begin().await?;
    for posting in &kept {
        let Some(id) = posting.get("id").and_then(Value::as_str) else {
            continue;
        };
        let source = posting.get("source").and_then(Value::as_str).unwrap_or("");
        let scan_source = posting
            .get("_scan_source")
            .and_then(Value::as_str)
            .unwrap_or(source);
        *kept_per_source.entry(scan_source.to_owned()).or_default() += 1;

        let existing: Option<(Option<String>, Option<String>)> =
            sqlx::query_as("SELECT first_seen_at, retired_at FROM postings WHERE id = ?")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;
        if existing
            .as_ref()
            .is_some_and(|(_, retired_at)| retired_at.is_some())
        {
            // The same identity never comes back after it expires.
            continue;
        }
        let first_seen = existing
            .and_then(|(first, _)| first)
            .unwrap_or_else(|| now_text.clone());
        let posted = posting
            .get("posted")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let derived = posted.is_none().then(|| {
            posting
                .get("age_days")
                .and_then(Value::as_i64)
                .map(|age| (today - Duration::days(age)).format("%Y-%m-%d").to_string())
        });
        let derived = derived.flatten();
        let expires = expiry(
            posted.as_deref().or(derived.as_deref()),
            &first_seen,
            window,
            today,
        );
        let Some(expires) = expires else {
            continue;
        };
        if expires <= today {
            retire(&mut tx, id, source, posting, &now_text, expires).await?;
            continue;
        }
        let salary_annual_min = posting.get("salary_annual_min").and_then(Value::as_i64);
        let salary_annual_max = posting.get("salary_annual_max").and_then(Value::as_i64);
        let flags = |key: &str| posting.get(key).and_then(Value::as_bool).unwrap_or(false) as i64;
        let description = posting.get("description").and_then(Value::as_str);
        let has_text = description.is_some_and(|text| !text.trim().is_empty()) as i64;
        let keywords = posting
            .get("keywords")
            .cloned()
            .unwrap_or(json!([]))
            .to_string();
        let skills_have = posting
            .get("skills_have")
            .cloned()
            .unwrap_or(json!([]))
            .to_string();
        let skills_gap = posting
            .get("skills_gap")
            .cloned()
            .unwrap_or(json!([]))
            .to_string();
        let emails = posting
            .get("emails")
            .cloned()
            .unwrap_or(json!([]))
            .to_string();
        let verified = posting
            .get("verified_emails")
            .cloned()
            .unwrap_or(json!([]))
            .to_string();

        sqlx::query(
            "INSERT INTO postings (id, source, ats, raw_id, company, title, location, url, salary_raw,
                salary_annual_min, salary_annual_max, salary_estimated, below_comp_floor, posted,
                posted_derived, age_days, scanned_at, tier, score, level_ii, requires_us_work_auth,
                keywords_json, skills_have_json, skills_gap_json, emails_json, verified_emails_json,
                has_text, first_seen_at, last_seen_at, expires_on, retired_at)
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
             ON CONFLICT(id) DO UPDATE SET
                source=excluded.source, ats=excluded.ats, raw_id=excluded.raw_id,
                company=excluded.company, title=excluded.title, location=excluded.location,
                url=excluded.url, salary_raw=excluded.salary_raw,
                salary_annual_min=excluded.salary_annual_min,
                salary_annual_max=excluded.salary_annual_max,
                salary_estimated=excluded.salary_estimated, below_comp_floor=excluded.below_comp_floor,
                posted=excluded.posted, posted_derived=excluded.posted_derived,
                age_days=excluded.age_days, scanned_at=excluded.scanned_at, tier=excluded.tier,
                score=excluded.score, level_ii=excluded.level_ii,
                requires_us_work_auth=excluded.requires_us_work_auth,
                has_text=MAX(postings.has_text, excluded.has_text),
                keywords_json=CASE WHEN excluded.has_text=1 THEN excluded.keywords_json ELSE postings.keywords_json END,
                skills_have_json=CASE WHEN excluded.has_text=1 THEN excluded.skills_have_json ELSE postings.skills_have_json END,
                skills_gap_json=CASE WHEN excluded.has_text=1 THEN excluded.skills_gap_json ELSE postings.skills_gap_json END,
                emails_json=CASE WHEN excluded.has_text=1 THEN excluded.emails_json ELSE postings.emails_json END,
                verified_emails_json=CASE WHEN excluded.has_text=1 THEN excluded.verified_emails_json ELSE postings.verified_emails_json END,
                last_seen_at=excluded.last_seen_at, expires_on=excluded.expires_on,
                retired_at=NULL",
        )
        .bind(id)
        .bind(source)
        .bind(posting.get("ats").and_then(Value::as_str))
        .bind(posting.get("raw_id").and_then(Value::as_str))
        .bind(posting.get("company").and_then(Value::as_str).unwrap_or(""))
        .bind(posting.get("title").and_then(Value::as_str).unwrap_or(""))
        .bind(posting.get("location").and_then(Value::as_str))
        .bind(posting.get("url").and_then(Value::as_str))
        .bind(posting.get("salary").and_then(Value::as_str))
        .bind(salary_annual_min)
        .bind(salary_annual_max)
        .bind(flags("salary_estimated"))
        .bind(flags("below_comp_floor"))
        .bind(posted.as_deref())
        .bind(derived.is_some() as i64)
        .bind(posting.get("age_days").and_then(Value::as_i64))
        .bind(&now_text)
        .bind(posting.get("tier").and_then(Value::as_str))
        .bind(posting.get("score").and_then(Value::as_i64))
        .bind(flags("level_ii"))
        .bind(flags("requires_us_work_auth"))
        .bind(&keywords)
        .bind(&skills_have)
        .bind(&skills_gap)
        .bind(&emails)
        .bind(&verified)
        .bind(has_text)
        .bind(&first_seen)
        .bind(&now_text)
        .bind(expires.format("%Y-%m-%d").to_string())
        .bind(Option::<String>::None)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO seen(id, reported, first_seen_at, last_seen_at) VALUES(?, 0, ?, ?)
             ON CONFLICT(id) DO UPDATE SET last_seen_at=excluded.last_seen_at",
        )
        .bind(id)
        .bind(&first_seen)
        .bind(&now_text)
        .execute(&mut *tx)
        .await?;

        if let Some(text) = description.filter(|text| !text.trim().is_empty()) {
            sqlx::query(
                "INSERT INTO descriptions(posting_id, text, source, fetched_at) VALUES(?,?,?,?)
                 ON CONFLICT(posting_id) DO UPDATE SET text=excluded.text, fetched_at=excluded.fetched_at",
            )
            .bind(id)
            .bind(text)
            .bind(source)
            .bind(&now_text)
            .execute(&mut *tx)
            .await?;
        }
    }

    // Retire rows that were live yesterday and are past their expiry today.
    sqlx::query(
        "UPDATE postings SET retired_at = ?
         WHERE retired_at IS NULL AND expires_on IS NOT NULL AND expires_on <= ?",
    )
    .bind(&now_text)
    .bind(today.format("%Y-%m-%d").to_string())
    .execute(&mut *tx)
    .await?;

    for (source, kept_count) in &kept_per_source {
        sqlx::query("UPDATE source_runs SET kept = ? WHERE scan_run_id = ? AND source = ?")
            .bind(kept_count)
            .bind(run_id)
            .bind(source)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;

    let stats = json!({
        "sources": outcome.per_source,
        "source_seconds": outcome.source_seconds,
        "collected": outcome.collected,
        "matched_filters": outcome.matched,
        "dropped": outcome.dropped,
        "tier_counts": outcome.tier_counts,
        "ran_at": now_text,
    });
    finish_run(pool, run_id, "ok", outcome.matched as i64, &stats).await?;
    let _ = events;
    Ok(outcome)
}

fn expiry(
    posted: Option<&str>,
    first_seen: &str,
    window: i64,
    today: NaiveDate,
) -> Option<NaiveDate> {
    let base = posted
        .and_then(|text| NaiveDate::parse_from_str(&text[..10.min(text.len())], "%Y-%m-%d").ok())
        .or_else(|| {
            first_seen
                .get(..10)
                .and_then(|text| NaiveDate::parse_from_str(text, "%Y-%m-%d").ok())
        })
        .unwrap_or(today);
    Some(base + Duration::days(window))
}

async fn retire(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: &str,
    source: &str,
    posting: &Value,
    now: &str,
    expires: NaiveDate,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT OR IGNORE INTO postings (id, source, company, title, location, url,
            first_seen_at, last_seen_at, expires_on, retired_at)
         VALUES (?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(id)
    .bind(source)
    .bind(posting.get("company").and_then(Value::as_str).unwrap_or(""))
    .bind(posting.get("title").and_then(Value::as_str).unwrap_or(""))
    .bind(posting.get("location").and_then(Value::as_str))
    .bind(posting.get("url").and_then(Value::as_str))
    .bind(now)
    .bind(now)
    .bind(expires.format("%Y-%m-%d").to_string())
    .bind(now)
    .execute(&mut **tx)
    .await?;
    sqlx::query("UPDATE postings SET retired_at = ? WHERE id = ? AND retired_at IS NULL")
        .bind(now)
        .bind(id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn finish_run(
    pool: &SqlitePool,
    run_id: i64,
    status: &str,
    matched: i64,
    stats: &Value,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE scan_runs SET status = ?, finished_at = ?, matched = ?, stats_json = ? WHERE id = ?",
    )
    .bind(status)
    .bind(Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true))
    .bind(matched)
    .bind(stats.to_string())
    .bind(run_id)
    .execute(pool)
    .await?;
    Ok(())
}

fn read_yaml(path: &Path) -> Option<Yaml> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_yaml::from_str(&text).ok()
}

/// A feed that hands over employer boards is half-used until the probe runs.
/// Say so every scan, because nothing in the feed's own output reveals it.
async fn report_unprobed(pool: &SqlitePool, root: &Path) {
    let probe_kinds = ["greenhouse", "ashby", "lever", "workable"];
    let config = read_yaml(&root.join("data/config/ats_boards.yaml")).unwrap_or(Yaml::Null);
    let mut known = std::collections::HashSet::new();
    if let Some(map) = config.as_mapping() {
        for (key, rows) in map {
            let Some(key) = key.as_str().filter(|key| key.starts_with("boards_")) else {
                continue;
            };
            let default_ats = &key["boards_".len()..];
            for row in rows.as_sequence().into_iter().flatten() {
                let ats = row.get("ats").and_then(Yaml::as_str).unwrap_or(default_ats);
                if let Some(slug) = row.get("slug").and_then(Yaml::as_str) {
                    known.insert(format!("{ats}|{slug}"));
                }
            }
        }
    }
    let Ok(rows) = sqlx::query("SELECT key FROM harvested_boards")
        .fetch_all(pool)
        .await
    else {
        return;
    };
    let mut gap = Vec::new();
    for row in rows {
        let key: String = row.get("key");
        let Some((ats, slug)) = key.split_once('|') else {
            continue;
        };
        if probe_kinds.contains(&ats) && !known.contains(&key) {
            gap.push(format!("{ats}/{slug}"));
        }
    }
    if !gap.is_empty() {
        let shown = gap.iter().take(10).cloned().collect::<Vec<_>>().join(", ");
        let more = if gap.len() > 10 {
            format!(" (+{} more)", gap.len() - 10)
        } else {
            String::new()
        };
        eprintln!(
            "[scan] {} harvested boards are not in ats_boards.yaml -- run `jh probe-ats`: {shown}{more}",
            gap.len()
        );
    }
}
