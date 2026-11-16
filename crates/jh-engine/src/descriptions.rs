//! Posting text for the brief engine. The dashboard never fetches; the scan
//! fills this cache, bounded per run, and reuses it forever.

use anyhow::Result;
use futures::stream::{self, StreamExt};
use jh_core::posting::apply_signature;
use jh_scan::ScanContext;
use regex::Regex;
use serde_json::Value;
use sqlx::SqlitePool;
use std::{collections::HashMap, sync::OnceLock, time::Duration};

pub const DESC_LIMIT: usize = 6000;
pub const DESC_BUDGET: usize = 250;
const FETCH_WORKERS: usize = 8;

pub async fn harvest(
    pool: &SqlitePool,
    ctx: &ScanContext,
    postings: &mut [Value],
    budget: usize,
) -> Result<()> {
    let cached: HashMap<String, String> =
        sqlx::query_as::<_, (String, String)>("SELECT posting_id, text FROM descriptions")
            .fetch_all(pool)
            .await
            .unwrap_or_default()
            .into_iter()
            .collect();

    let mut todo = Vec::new();
    for (index, posting) in postings.iter_mut().enumerate() {
        let Some(id) = posting.get("id").and_then(Value::as_str).map(str::to_owned) else {
            continue;
        };
        if let Some(text) = cached.get(&id) {
            posting["description"] = Value::String(text.clone());
            continue;
        }
        if posting
            .get("description")
            .and_then(Value::as_str)
            .is_some_and(|text| !text.trim().is_empty())
        {
            continue;
        }
        if fetcher(posting).is_some() {
            todo.push(index);
        }
    }
    todo.truncate(budget);
    if todo.is_empty() {
        return Ok(());
    }

    let results: Vec<(usize, Option<String>)> = stream::iter(todo.into_iter().map(|index| {
        let ctx = ctx.clone();
        let posting = postings[index].clone();
        async move {
            let text = fetch(&ctx, &posting).await;
            (index, text)
        }
    }))
    .buffer_unordered(FETCH_WORKERS)
    .collect()
    .await;

    let mut fetched = 0;
    for (index, text) in results {
        if let Some(text) = text {
            postings[index]["description"] = Value::String(text);
            fetched += 1;
        }
    }
    eprintln!(
        "[scan] descriptions: {fetched} fetched, {} skipped by the budget",
        postings
            .iter()
            .filter(|posting| {
                posting
                    .get("description")
                    .and_then(Value::as_str)
                    .is_none_or(|text| text.trim().is_empty())
            })
            .count()
    );
    Ok(())
}

fn fetcher(posting: &Value) -> Option<&'static str> {
    match posting.get("ats").and_then(Value::as_str) {
        Some("workday") => Some("workday"),
        Some("greenhouse") => Some("greenhouse"),
        Some("lever") => Some("lever"),
        Some("ashby") => Some("ashby"),
        _ => None,
    }
}

async fn fetch(ctx: &ScanContext, posting: &Value) -> Option<String> {
    match fetcher(posting)? {
        "workday" => fetch_workday(ctx, posting).await,
        "greenhouse" => fetch_greenhouse(ctx, posting).await,
        "lever" => fetch_lever(ctx, posting).await,
        "ashby" => fetch_ashby(ctx, posting).await,
        _ => None,
    }
}

fn clean(text: &str) -> Option<String> {
    let text = jh_scan::sources::strip_tags(text);
    let text: String = text.trim().chars().take(DESC_LIMIT).collect();
    (!text.is_empty()).then_some(text)
}

async fn fetch_workday(ctx: &ScanContext, posting: &Value) -> Option<String> {
    static WORKDAY: OnceLock<Regex> = OnceLock::new();
    let url = posting.get("url").and_then(Value::as_str)?;
    let capture = WORKDAY
        .get_or_init(|| {
            Regex::new(
                r"(?i)^https?://([a-z0-9-]+)\.(wd\d+)\.myworkdayjobs\.com/(?:[a-z]{2}-[A-Z]{2}/)?([^/?#]+)(/.*)$",
            )
            .unwrap()
        })
        .captures(url)?;
    let host = format!("{}.{}.myworkdayjobs.com", &capture[1], &capture[2]);
    let tenant = &capture[1];
    let site = &capture[3];
    let path = capture[4].trim_end_matches('/');
    let raw = ctx
        .http
        .get_text_with_timeout(
            &format!("https://{host}/wday/cxs/{tenant}/{site}{path}"),
            &[],
            Duration::from_secs(25),
            1,
        )
        .await?;
    let value: Value = serde_json::from_str(&raw).ok()?;
    clean(
        value
            .get("jobPostingInfo")
            .and_then(|info| info.get("jobDescription"))
            .and_then(Value::as_str)
            .unwrap_or(""),
    )
}

async fn fetch_greenhouse(ctx: &ScanContext, posting: &Value) -> Option<String> {
    let url = posting.get("url").and_then(Value::as_str)?;
    let (vendor, slug, job_id, _) = apply_signature(url)?;
    if vendor != "greenhouse" {
        return None;
    }
    let raw = ctx
        .http
        .get_text_with_timeout(
            &format!(
                "https://boards-api.greenhouse.io/v1/boards/{slug}/jobs/{job_id}?content=true"
            ),
            &[],
            Duration::from_secs(25),
            1,
        )
        .await?;
    let value: Value = serde_json::from_str(&raw).ok()?;
    clean(value.get("content").and_then(Value::as_str).unwrap_or(""))
}

async fn fetch_lever(ctx: &ScanContext, posting: &Value) -> Option<String> {
    let url = posting.get("url").and_then(Value::as_str)?;
    let (vendor, slug, job_id, _) = apply_signature(url)?;
    if vendor != "lever" {
        return None;
    }
    let raw = ctx
        .http
        .get_text_with_timeout(
            &format!("https://api.lever.co/v0/postings/{slug}/{job_id}?mode=json"),
            &[],
            Duration::from_secs(25),
            1,
        )
        .await?;
    let value: Value = serde_json::from_str(&raw).ok()?;
    clean(
        value
            .get("descriptionPlain")
            .or_else(|| value.get("description"))
            .and_then(Value::as_str)
            .unwrap_or(""),
    )
}

async fn fetch_ashby(ctx: &ScanContext, posting: &Value) -> Option<String> {
    let url = posting.get("url").and_then(Value::as_str)?;
    let (vendor, slug, job_id, _) = apply_signature(url)?;
    if vendor != "ashby" {
        return None;
    }
    let raw = ctx
        .http
        .get_text_with_timeout(
            &format!("https://api.ashbyhq.com/posting-api/job-board/{slug}"),
            &[],
            Duration::from_secs(30),
            1,
        )
        .await?;
    let value: Value = serde_json::from_str(&raw).ok()?;
    for job in value.get("jobs").and_then(Value::as_array)? {
        let id = job
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| {
                job.get("id")
                    .and_then(Value::as_i64)
                    .map(|id| id.to_string())
            });
        if id.as_deref() == Some(job_id.as_str()) {
            return clean(
                job.get("descriptionPlain")
                    .or_else(|| job.get("descriptionHtml"))
                    .and_then(Value::as_str)
                    .unwrap_or(""),
            );
        }
    }
    None
}
