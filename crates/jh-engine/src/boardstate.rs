//! Durable per-board state in SQLite: what answered, what failed, when to look
//! again, and the sweep cache the daily scan reads instead of re-sweeping.

use anyhow::Result;
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use jh_scan::BoardNote;
use regex::Regex;
use serde_json::Value;
use sqlx::{Row, SqlitePool};
use std::sync::OnceLock;

pub const RETRY_BACKOFF_DAYS_TRANSIENT: [i64; 3] = [1, 3, 14];
pub const RETRY_BACKOFF_DAYS_STRUCTURAL: [i64; 2] = [7, 30];
pub const BROKEN_RECHECK_DAYS: i64 = 30;
pub const SWEEP_MAX_FAILURE_FRACTION: f64 = 0.25;
pub const SWEEP_MIN_FAILURES: usize = 60;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct BoardRow {
    pub key: String,
    pub vendor: Option<String>,
    pub slug: Option<String>,
    pub tenant: Option<String>,
    pub wd: Option<String>,
    pub site: Option<String>,
    pub status: Option<String>,
    pub consecutive_failures: i64,
    pub last_ok_at: Option<String>,
    pub last_checked_at: Option<String>,
    pub next_check_at: Option<String>,
    pub last_error: Option<String>,
    pub error_kind: Option<String>,
    pub updated_at: String,
}

pub fn classify(error: &str) -> &'static str {
    static STRUCTURAL: OnceLock<Regex> = OnceLock::new();
    if STRUCTURAL
        .get_or_init(|| Regex::new(r"(?i)\b(404|422)\b|not found|unprocessable").unwrap())
        .is_match(error)
    {
        "structural"
    } else {
        "transient"
    }
}

pub fn schedule(kind: &str, consecutive: i64) -> (&'static str, i64) {
    let days = match kind {
        "structural" => RETRY_BACKOFF_DAYS_STRUCTURAL
            .get(consecutive.saturating_sub(1) as usize)
            .copied(),
        _ => RETRY_BACKOFF_DAYS_TRANSIENT
            .get(consecutive.saturating_sub(1) as usize)
            .copied(),
    };
    match days {
        Some(days) => ("failing", days),
        None => ("broken", BROKEN_RECHECK_DAYS),
    }
}

/// A sweep that loses a large share of its boards is the throttle talking.
pub fn sweep_verdict(notes: &[BoardNote], previous_postings: usize) -> (bool, String) {
    let total = notes.len();
    if total == 0 {
        return (false, "no board outcomes recorded".into());
    }
    let failed = notes.iter().filter(|note| !note.ok).count();
    if failed >= SWEEP_MIN_FAILURES && (failed as f64 / total as f64) > SWEEP_MAX_FAILURE_FRACTION {
        return (
            false,
            format!(
                "{failed} of {total} boards failed ({:.0}%) -- that is the throttle or the network, not {failed} broken configs",
                failed as f64 / total as f64 * 100.0
            ),
        );
    }
    let productive = notes.iter().filter(|note| note.ok && note.hits > 0).count();
    if previous_postings > 0 && productive == 0 {
        return (
            false,
            format!(
                "no board returned a posting where the last sweep found {previous_postings}; treating this sweep as lost"
            ),
        );
    }
    (true, format!("{failed} of {total} boards failed"))
}

pub async fn write_notes(pool: &SqlitePool, notes: &[BoardNote]) -> Result<()> {
    let now = Utc::now();
    let stamp = now.to_rfc3339_opts(SecondsFormat::Secs, true);
    for note in notes {
        let existing = sqlx::query_as::<_, BoardRow>(
            "SELECT key,vendor,slug,tenant,wd,site,status,consecutive_failures,last_ok_at,
                last_checked_at,next_check_at,last_error,error_kind,updated_at
             FROM board_health WHERE key = ?",
        )
        .bind(&note.key)
        .fetch_optional(pool)
        .await?;
        let parts: Vec<&str> = note.key.split('|').collect();
        let (vendor, slug, tenant, wd, site) = if parts.len() >= 3 {
            (None, None, Some(parts[0]), Some(parts[1]), Some(parts[2]))
        } else {
            (
                Some(parts.first().copied().unwrap_or("")),
                Some(parts.get(1).copied().unwrap_or("")),
                None,
                None,
                None,
            )
        };
        let attempts: i64 = sqlx::query("SELECT attempts FROM board_health WHERE key = ?")
            .bind(&note.key)
            .fetch_optional(pool)
            .await?
            .map(|row| row.get::<i64, _>("attempts"))
            .unwrap_or(0)
            + 1;
        let (status, consecutive, last_ok, next_check, error, kind, hits) = if note.ok {
            (
                "ok",
                0,
                Some(stamp.clone()),
                None,
                None,
                None,
                note.hits as i64,
            )
        } else {
            let error = note.error.clone().unwrap_or_else(|| "no answer".to_owned());
            let kind = classify(&error);
            let previous = existing
                .as_ref()
                .map(|row| row.consecutive_failures)
                .unwrap_or(0);
            let consecutive = previous + 1;
            let (status, days) = schedule(kind, consecutive);
            let next = (now + Duration::days(days)).to_rfc3339_opts(SecondsFormat::Secs, true);
            (
                status,
                consecutive,
                existing.as_ref().and_then(|row| row.last_ok_at.clone()),
                Some(next),
                Some(error),
                Some(kind),
                0,
            )
        };
        sqlx::query(
            "INSERT INTO board_health(key,vendor,slug,tenant,wd,site,status,consecutive_failures,
                last_ok_at,last_checked_at,next_check_at,last_error,error_kind,updated_at,
                attempts,hits_last_sweep,cursor_json)
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
             ON CONFLICT(key) DO UPDATE SET
                vendor=excluded.vendor, slug=excluded.slug, tenant=excluded.tenant,
                wd=excluded.wd, site=excluded.site, status=excluded.status,
                consecutive_failures=excluded.consecutive_failures,
                last_ok_at=excluded.last_ok_at, last_checked_at=excluded.last_checked_at,
                next_check_at=excluded.next_check_at, last_error=excluded.last_error,
                error_kind=excluded.error_kind, updated_at=excluded.updated_at,
                attempts=excluded.attempts, hits_last_sweep=excluded.hits_last_sweep,
                cursor_json=COALESCE(excluded.cursor_json, board_health.cursor_json)",
        )
        .bind(&note.key)
        .bind(vendor)
        .bind(slug)
        .bind(tenant)
        .bind(wd)
        .bind(site)
        .bind(status)
        .bind(consecutive)
        .bind(last_ok)
        .bind(&stamp)
        .bind(next_check)
        .bind(error)
        .bind(kind)
        .bind(&stamp)
        .bind(attempts)
        .bind(hits)
        .bind(note.cursor.as_ref().map(serde_json::Value::to_string))
        .execute(pool)
        .await?;
    }
    Ok(())
}

pub async fn summary(pool: &SqlitePool) -> Result<(i64, i64, i64, i64)> {
    let rows = sqlx::query("SELECT status, COUNT(*) AS n FROM board_health GROUP BY status")
        .fetch_all(pool)
        .await?;
    let mut known = 0;
    let (mut ok, mut failing, mut broken) = (0, 0, 0);
    for row in rows {
        let n: i64 = row.get("n");
        known += n;
        match row.get::<Option<String>, _>("status").as_deref() {
            Some("ok") => ok += n,
            Some("broken") => broken += n,
            _ => failing += n,
        }
    }
    Ok((known, ok, failing, broken))
}

pub async fn line(pool: &SqlitePool) -> Result<String> {
    let (known, ok, failing, broken) = summary(pool).await?;
    if known == 0 {
        return Ok("board state: empty".into());
    }
    Ok(format!(
        "board state: {known} known, {ok} ok, {failing} failing, {broken} broken"
    ))
}

/// Boards whose backoff has come up. `force` ignores the schedule but still
/// returns only failing and broken boards.
pub async fn due(pool: &SqlitePool, force: bool) -> Result<Vec<BoardRow>> {
    let rows = sqlx::query_as::<_, BoardRow>(
        "SELECT key,vendor,slug,tenant,wd,site,status,consecutive_failures,last_ok_at,
            last_checked_at,next_check_at,last_error,error_kind,updated_at
         FROM board_health WHERE status <> 'ok' ORDER BY key",
    )
    .fetch_all(pool)
    .await?;
    if force {
        return Ok(rows);
    }
    let now = Utc::now();
    Ok(rows
        .into_iter()
        .filter(|row| {
            row.next_check_at
                .as_deref()
                .and_then(parse_stamp)
                .is_none_or(|at| at <= now)
        })
        .collect())
}

pub fn parse_stamp(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|at| at.with_timezone(&Utc))
}

/// Store one sweep's postings and its timestamp in the `cache` table.
pub async fn write_sweep(pool: &SqlitePool, postings: &[Value]) -> Result<()> {
    let stamp = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    let payload = serde_json::json!({
        "swept_at": stamp,
        "count": postings.len(),
        "postings": postings,
    });
    sqlx::query(
        "INSERT INTO cache(key,value_json,updated_at) VALUES('workday_sweep',?,?)
         ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json, updated_at=excluded.updated_at",
    )
    .bind(payload.to_string())
    .bind(&stamp)
    .execute(pool)
    .await?;
    Ok(())
}

/// The cached sweep as postings, or None when the cache is absent or older
/// than `max_age_hours`. Ages advance by whole days, so a cache never makes a
/// posting look fresher than it is.
pub async fn read_sweep(pool: &SqlitePool, max_age_hours: f64) -> Result<Option<Vec<Value>>> {
    let Some(row) = sqlx::query("SELECT value_json FROM cache WHERE key = 'workday_sweep'")
        .fetch_optional(pool)
        .await?
    else {
        return Ok(None);
    };
    let payload: Value = serde_json::from_str(row.get("value_json")).unwrap_or(Value::Null);
    let Some(swept) = payload
        .get("swept_at")
        .and_then(Value::as_str)
        .and_then(parse_stamp)
    else {
        return Ok(None);
    };
    let now = Utc::now();
    let age_hours = (now - swept).num_seconds() as f64 / 3600.0;
    if age_hours > max_age_hours {
        return Ok(None);
    }
    let drift = (now.date_naive() - swept.date_naive()).num_days();
    let postings = payload
        .get("postings")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|mut posting| {
            if drift > 0
                && let Some(age) = posting.get("age_days").and_then(Value::as_i64)
            {
                posting["age_days"] = serde_json::json!(age + drift);
            }
            posting
        })
        .collect();
    eprintln!("[scan] workday: cached sweep is {age_hours:.1}h old");
    Ok(Some(postings))
}
