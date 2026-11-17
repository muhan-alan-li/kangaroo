//! The default brief engine: locate the posting, run stages A → B → C,
//! validate the edits, and persist the result in SQLite.

use crate::{stages, validate};
use anyhow::Result;
use chrono::{SecondsFormat, Utc};
use serde_json::{Value, json};
use sqlx::{Row, SqlitePool};
use std::{path::Path, time::Instant};

pub async fn generate(pool: &SqlitePool, root: &Path, posting_id: &str) -> Value {
    let started = Instant::now();
    let Some(row) = read_posting(pool, posting_id).await else {
        let payload = json!({
            "status": "failed",
            "error": format!("no posting {posting_id} in the database"),
            "generated_at": now(),
        });
        let _ = write_result(pool, posting_id, &payload, "failed", None).await;
        return payload;
    };
    let (text, description_source) = description(pool, posting_id).await;
    let resume: String = std::fs::read_to_string(root.join("data/candidate/resume-text.md"))
        .unwrap_or_else(|_| "(resume text is unavailable)".into());
    let _ = mark_running(pool, posting_id).await;

    let requirements = match stages::requirements(root, &row, &text).await {
        Ok(value) => value,
        Err(error) => {
            return failed(pool, posting_id, &error, &description_source, started).await;
        }
    };
    let matches = match stages::matches(root, &row, &text, &requirements, &resume).await {
        Ok(value) => value,
        Err(error) => {
            return failed(pool, posting_id, &error, &description_source, started).await;
        }
    };
    let _ = write_raw(pool, posting_id, &requirements, &matches).await;
    let written = match stages::brief(root, &row, &text, &requirements, &matches, &resume).await {
        Ok(value) => value,
        Err(error) => {
            return failed(pool, posting_id, &error, &description_source, started).await;
        }
    };

    let mut tailoring = written
        .get("tailoring")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let rewords = tailoring
        .get("reword")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let (kept, dropped, flags) = validate::check_rewords(&rewords, &resume, &text);
    tailoring.insert("reword".into(), Value::Array(kept));
    let mut payload = written.as_object().cloned().unwrap_or_default();
    payload.insert("tailoring".into(), Value::Object(tailoring));
    if !dropped.is_empty() {
        payload.insert("reword_dropped".into(), Value::Array(dropped));
    }
    if !flags.is_empty() {
        payload.insert(
            "reword_flags".into(),
            Value::Array(flags.into_iter().map(Value::String).collect()),
        );
    }
    payload.insert("status".into(), Value::String("ok".into()));
    payload.insert(
        "description_source".into(),
        Value::String(description_source.clone()),
    );
    payload.insert(
        "elapsed_s".into(),
        json!(((started.elapsed().as_secs_f64() * 10.0).round()) / 10.0),
    );
    payload.insert(
        "company".into(),
        row.get("company").cloned().unwrap_or(Value::Null),
    );
    payload.insert(
        "title".into(),
        row.get("title").cloned().unwrap_or(Value::Null),
    );
    payload.insert("url".into(), row.get("url").cloned().unwrap_or(Value::Null));
    payload.insert("generated_at".into(), Value::String(now()));
    let payload = Value::Object(payload);
    let _ = write_result(pool, posting_id, &payload, "ok", None).await;
    payload
}

async fn failed(
    pool: &SqlitePool,
    posting_id: &str,
    error: &anyhow::Error,
    description_source: &str,
    started: Instant,
) -> Value {
    let payload = json!({
        "status": "failed",
        "error": format!("{error:#}"),
        "description_source": description_source,
        "stage": "tailoring chain",
        "elapsed_s": ((started.elapsed().as_secs_f64() * 10.0).round()) / 10.0,
        "generated_at": now(),
    });
    let _ = write_result(
        pool,
        posting_id,
        &payload,
        "failed",
        Some(&format!("{error:#}")),
    )
    .await;
    payload
}

async fn read_posting(pool: &SqlitePool, posting_id: &str) -> Option<Value> {
    let row = sqlx::query(
        "SELECT id, company, title, location, url, posted, tier, score FROM postings WHERE id = ?",
    )
    .bind(posting_id)
    .fetch_optional(pool)
    .await
    .ok()??;
    Some(json!({
        "id": row.get::<String, _>("id"),
        "company": row.get::<Option<String>, _>("company"),
        "title": row.get::<Option<String>, _>("title"),
        "location": row.get::<Option<String>, _>("location"),
        "url": row.get::<Option<String>, _>("url"),
        "posted": row.get::<Option<String>, _>("posted"),
        "tier": row.get::<Option<String>, _>("tier"),
        "score": row.get::<Option<i64>, _>("score"),
    }))
}

async fn description(pool: &SqlitePool, posting_id: &str) -> (String, String) {
    let row = sqlx::query("SELECT text FROM descriptions WHERE posting_id = ?")
        .bind(posting_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    match row.and_then(|row| row.try_get::<String, _>("text").ok()) {
        Some(text) if !text.trim().is_empty() => (text, "cache".into()),
        _ => (String::new(), "none".into()),
    }
}

async fn mark_running(pool: &SqlitePool, posting_id: &str) -> Result<()> {
    let stamp = now();
    sqlx::query(
        "INSERT INTO briefs(posting_id,status,engine,generated_at,started_at,payload_json,error)
         VALUES(?, 'running', 'deepseek', NULL, ?, '{}', NULL)
         ON CONFLICT(posting_id) DO UPDATE SET status='running', engine='deepseek',
            generated_at=NULL, started_at=excluded.started_at, payload_json='{}', error=NULL",
    )
    .bind(posting_id)
    .bind(&stamp)
    .execute(pool)
    .await?;
    Ok(())
}

async fn write_raw(
    pool: &SqlitePool,
    posting_id: &str,
    requirements: &Value,
    matches: &Value,
) -> Result<()> {
    let payload = json!({"stage_a": requirements, "stage_b": matches});
    sqlx::query(
        "INSERT INTO brief_artifacts(posting_id,kind,payload_json,created_at) VALUES(?, 'raw', ?, ?)
         ON CONFLICT(posting_id,kind) DO UPDATE SET payload_json=excluded.payload_json, created_at=excluded.created_at",
    )
    .bind(posting_id)
    .bind(payload.to_string())
    .bind(now())
    .execute(pool)
    .await?;
    Ok(())
}

async fn write_result(
    pool: &SqlitePool,
    posting_id: &str,
    payload: &Value,
    status: &str,
    error: Option<&str>,
) -> Result<()> {
    let stamp = now();
    sqlx::query(
        "INSERT INTO briefs(posting_id,status,engine,generated_at,started_at,payload_json,error)
         VALUES(?,?, 'deepseek', ?, NULL, ?, ?)
         ON CONFLICT(posting_id) DO UPDATE SET status=excluded.status,
            engine=excluded.engine, generated_at=excluded.generated_at,
            payload_json=excluded.payload_json, error=excluded.error",
    )
    .bind(posting_id)
    .bind(status)
    .bind(&stamp)
    .bind(payload.to_string())
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}
