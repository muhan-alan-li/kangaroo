//! The `ats_boards` source: public Greenhouse, Ashby and Lever board APIs.
//!
//! The board lists stay in `data/config/*.yaml`; this module reads the keys
//! beginning with `boards_`, and the row's own `ats` field decides the parser.

use super::{Source, parse_array, yaml_str, yaml_string_list};
use crate::ScanContext;
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use futures::stream::{self, StreamExt};
use kangaroo_core::parsing::parse_age_days;
use kangaroo_core::posting::make_posting;
use serde_json::Value;
use serde_yaml::Value as Yaml;
use std::time::Duration;

pub struct AtsBoards;

const BOARD_CONCURRENCY: usize = 8;

#[derive(Debug, Clone)]
pub struct Board {
    pub name: String,
    pub ats: String,
    pub slug: String,
    pub tenant: String,
    pub wd: String,
    pub site: String,
}

#[async_trait]
impl Source for AtsBoards {
    fn id(&self) -> &'static str {
        "ats_boards"
    }
    async fn fetch(&self, ctx: &ScanContext, cfg: &Yaml, _filters: &Yaml) -> Result<Vec<Value>> {
        let entries = load_entries(ctx, cfg);
        let workday_boards: Vec<Board> = entries
            .iter()
            .filter(|board| {
                board.ats == "workday" && !board.tenant.is_empty() && !board.site.is_empty()
            })
            .cloned()
            .collect();
        let json_boards: Vec<Board> = entries
            .iter()
            .filter(|board| {
                super::jsonboards::VENDORS.contains(&board.ats.as_str()) && !board.slug.is_empty()
            })
            .cloned()
            .collect();
        let simple_boards: Vec<Board> = entries
            .iter()
            .filter(|board| {
                matches!(board.ats.as_str(), "greenhouse" | "ashby" | "lever")
                    && !board.slug.is_empty()
            })
            .cloned()
            .collect();
        let icims = entries.iter().filter(|board| board.ats == "icims").count();
        if icims > 0 {
            eprintln!("[scan] icims: {icims} boards skipped (not supported by the Rust scan)");
        }
        eprintln!(
            "[scan] ats: {} Workday, {} JSON-vendor, {} Greenhouse/Ashby/Lever boards",
            workday_boards.len(),
            json_boards.len(),
            simple_boards.len()
        );

        let workday_future = async {
            if let Some(cached) = &ctx.workday_cache {
                eprintln!(
                    "[scan] workday: {} postings from the sweep cache ({} boards not re-read)",
                    cached.len(),
                    workday_boards.len()
                );
                (cached.as_ref().clone(), 0)
            } else {
                super::workday::fetch_boards(ctx, &workday_boards, cfg).await
            }
        };
        let (workday, json, simple) = tokio::join!(
            workday_future,
            super::jsonboards::fetch_boards(ctx, &json_boards, cfg),
            fetch_simple(ctx, &simple_boards),
        );
        let mut out = workday.0;
        out.extend(json);
        out.extend(simple);
        Ok(out)
    }
}

async fn fetch_simple(ctx: &ScanContext, boards: &[Board]) -> Vec<Value> {
    let results: Vec<Vec<Value>> = stream::iter(boards.iter().cloned().map(|board| {
        let ctx = ctx.clone();
        async move { fetch_board(&ctx, &board).await }
    }))
    .buffer_unordered(BOARD_CONCURRENCY)
    .collect()
    .await;
    results.into_iter().flatten().collect()
}

pub fn load_entries(ctx: &ScanContext, cfg: &Yaml) -> Vec<Board> {
    let mut out = Vec::new();
    if let Some(companies) = cfg.get("companies").and_then(Yaml::as_sequence) {
        for row in companies {
            if let Some(board) = board_from_row(row, None) {
                out.push(board);
            }
        }
    }
    let mut files = Vec::new();
    if let Some(file) = yaml_str(cfg, "companies_file") {
        files.push(file.to_owned());
    }
    files.extend(yaml_string_list(cfg, "companies_files"));
    for file in files {
        let path = ctx.root.join(&file);
        let Ok(text) = std::fs::read_to_string(&path) else {
            eprintln!("[scan] ats companies_file missing: {}", path.display());
            continue;
        };
        let Ok(document) = serde_yaml::from_str::<Yaml>(&text) else {
            eprintln!("[scan] ats companies_file unreadable: {}", path.display());
            continue;
        };
        let Some(map) = document.as_mapping() else {
            continue;
        };
        for (key, rows) in map {
            let Some(key) = key.as_str().filter(|key| key.starts_with("boards_")) else {
                continue;
            };
            let default_ats = &key["boards_".len()..];
            let Some(rows) = rows.as_sequence() else {
                continue;
            };
            for row in rows {
                if let Some(board) = board_from_row(row, Some(default_ats)) {
                    out.push(board);
                }
            }
        }
    }
    out
}

fn board_from_row(row: &Yaml, default_ats: Option<&str>) -> Option<Board> {
    let ats = yaml_str(row, "ats")
        .or(default_ats)
        .unwrap_or("")
        .to_owned();
    let tenant = yaml_str(row, "tenant").unwrap_or("").to_owned();
    let wd = yaml_str(row, "wd").unwrap_or("").to_owned();
    let site = yaml_str(row, "site").unwrap_or("").to_owned();
    let mut slug = yaml_str(row, "slug").unwrap_or("").to_owned();
    if slug.is_empty() {
        let bits: Vec<&str> = [tenant.as_str(), wd.as_str(), site.as_str()]
            .into_iter()
            .filter(|field| !field.is_empty())
            .collect();
        if !bits.is_empty() {
            slug = bits.join("|");
        }
    }
    let name = yaml_str(row, "name")
        .filter(|name| !name.is_empty())
        .unwrap_or(&slug)
        .to_owned();
    (!ats.is_empty() && !slug.is_empty()).then_some(Board {
        name,
        ats,
        slug,
        tenant,
        wd,
        site,
    })
}

async fn fetch_board(ctx: &ScanContext, board: &Board) -> Vec<Value> {
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
        _ => return Vec::new(),
    };
    let Some(body) = ctx
        .http
        .get_text_with_timeout(&url, &[], Duration::from_secs(30), 2)
        .await
    else {
        return Vec::new();
    };
    match board.ats.as_str() {
        "greenhouse" => parse_greenhouse(&body, &board.name),
        "ashby" => parse_ashby(&body, &board.name),
        "lever" => parse_lever(&body, &board.name),
        _ => Vec::new(),
    }
}

pub fn parse_greenhouse(body: &str, company: &str) -> Vec<Value> {
    parse_jobs(body, Some("jobs"), "greenhouse", company, |job| {
        let posted = job
            .get("first_published")
            .or_else(|| job.get("updated_at"))
            .and_then(Value::as_str)
            .and_then(iso_date);
        let location = job
            .get("location")
            .and_then(|location| location.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("");
        Job {
            title: string_field(job, "title"),
            location: location.to_owned(),
            url: string_field(job, "absolute_url"),
            posted,
            raw_id: scalar_field(job, "id"),
            skip: false,
        }
    })
}

pub fn parse_ashby(body: &str, company: &str) -> Vec<Value> {
    parse_jobs(body, Some("jobs"), "ashby", company, |job| {
        if job.get("isListed").and_then(Value::as_bool) == Some(false) {
            return Job::skip();
        }
        let posted = job
            .get("publishedAt")
            .or_else(|| job.get("updatedAt"))
            .and_then(Value::as_str)
            .and_then(iso_date);
        Job {
            title: string_field(job, "title"),
            location: string_field(job, "location"),
            url: string_field(job, "jobUrl"),
            posted,
            raw_id: scalar_field(job, "id"),
            skip: false,
        }
    })
}

pub fn parse_lever(body: &str, company: &str) -> Vec<Value> {
    parse_jobs(body, None, "lever", company, |job| {
        let posted = job
            .get("createdAt")
            .and_then(Value::as_i64)
            .and_then(DateTime::<Utc>::from_timestamp_millis)
            .map(|at| at.date_naive());
        let location = job
            .get("categories")
            .and_then(|categories| categories.get("location"))
            .and_then(Value::as_str)
            .unwrap_or("");
        Job {
            title: string_field(job, "text"),
            location: location.to_owned(),
            url: string_field(job, "hostedUrl"),
            posted,
            raw_id: scalar_field(job, "id"),
            skip: false,
        }
    })
}

struct Job {
    title: String,
    location: String,
    url: String,
    posted: Option<NaiveDate>,
    raw_id: Option<String>,
    skip: bool,
}

impl Job {
    fn skip() -> Self {
        Self {
            title: String::new(),
            location: String::new(),
            url: String::new(),
            posted: None,
            raw_id: None,
            skip: true,
        }
    }
}

fn parse_jobs(
    body: &str,
    marker: Option<&str>,
    ats: &str,
    company: &str,
    read: impl Fn(&Value) -> Job,
) -> Vec<Value> {
    let jobs = parse_array(body, marker);
    let mut out = Vec::new();
    for job in jobs {
        let parsed = read(&job);
        if parsed.skip || parsed.title.is_empty() {
            continue;
        }
        let posted = parsed
            .posted
            .map(|date| date.format("%Y-%m-%d").to_string());
        out.push(make_posting(
            "ats",
            company,
            &parsed.title,
            &parsed.location,
            (!parsed.url.is_empty()).then_some(parsed.url.as_str()),
            posted.as_deref(),
            parse_age_days("", parsed.posted),
            None,
            Some(ats),
            parsed.raw_id.as_deref(),
            None,
            None,
        ));
    }
    out
}

fn string_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_owned()
}

fn scalar_field(value: &Value, key: &str) -> Option<String> {
    match value.get(key) {
        Some(Value::String(text)) => Some(text.clone()),
        Some(Value::Number(number)) => Some(number.to_string()),
        _ => None,
    }
}

fn iso_date(text: &str) -> Option<NaiveDate> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|at| at.date_naive())
}
