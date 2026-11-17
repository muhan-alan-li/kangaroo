//! The seven uniform JSON vendors: Workable, SmartRecruiters, Rippling,
//! BambooHR, Recruitee, Breezy and Teamtailor.

use super::ats::Board;
use super::workday::{region_ok, role_regex};
use super::{percent_encode, yaml_i64, yaml_string_list};
use crate::ScanContext;
use chrono::NaiveDate;
use futures::stream::{self, StreamExt};
use kangaroo_core::parsing::parse_age_days;
use kangaroo_core::posting::{apply_signature, make_posting};
use regex::Regex;
use serde_json::{Value, json};
use serde_yaml::Value as Yaml;
use std::{sync::OnceLock, time::Duration};

const PAGE_SIZE: usize = 100;

type BoardResult = (Board, Vec<(Row, Option<i64>)>, bool, usize, usize);

#[derive(Debug, Clone, Default)]
struct Row {
    title: String,
    location: String,
    country_code: Option<String>,
    posted: Option<NaiveDate>,
    url: Option<String>,
    raw_id: Option<String>,
}

pub async fn fetch_boards(ctx: &ScanContext, boards: &[Board], cfg: &Yaml) -> Vec<Value> {
    let opts = cfg.get("jsonboards").unwrap_or(&Yaml::Null);
    let workers = yaml_i64(opts, "workers").unwrap_or(12).max(1) as usize;
    let window = yaml_i64(opts, "window_days").unwrap_or(7);
    let max_pages = yaml_i64(opts, "max_pages").unwrap_or(2).max(1);
    let timeout = yaml_i64(opts, "timeout").unwrap_or(25).max(1) as u64;
    let role = role_regex(&yaml_string_list(opts, "title_words"));

    let results: Vec<BoardResult> = stream::iter(boards.iter().cloned().map(|board| {
        let ctx = ctx.clone();
        let role = role.clone();
        async move {
            let (rows, ok, dropped, undated) =
                fetch_board(&ctx, &board, window, timeout, max_pages, role).await;
            (board, rows, ok, dropped, undated)
        }
    }))
    .buffer_unordered(workers)
    .collect()
    .await;

    let mut out = Vec::new();
    let mut dead = 0;
    let mut filtered = 0;
    let mut undated = 0;
    let mut per_vendor = std::collections::BTreeMap::<String, usize>::new();
    for (board, rows, ok, dropped, no_date) in results {
        ctx.note_board(
            format!("{}|{}", board.ats, board.slug),
            Some(board.name.clone()),
            ok,
            rows.len(),
            None,
        );
        filtered += dropped;
        undated += no_date;
        if !ok {
            dead += 1;
            continue;
        }
        let company = board.name.trim();
        for (row, age) in rows {
            let dedupe = row
                .url
                .as_deref()
                .and_then(apply_signature)
                .map(|(_, _, _, key)| key)
                .or_else(|| {
                    row.raw_id
                        .as_ref()
                        .map(|id| format!("ats|{}:{}:{}", board.ats, board.slug.to_lowercase(), id))
                });
            let mut extra = json!({ "board": format!("{}|{}", board.ats, board.slug) });
            if age.is_none() {
                extra["date_missing"] = json!(true);
            }
            let posted = row.posted.map(|date| date.format("%Y-%m-%d").to_string());
            out.push(make_posting(
                "ats",
                company,
                &row.title,
                &row.location,
                row.url.as_deref(),
                posted.as_deref(),
                age,
                None,
                Some(&board.ats),
                row.raw_id.as_deref(),
                Some(&extra),
                dedupe.as_deref(),
            ));
            *per_vendor.entry(board.ats.clone()).or_default() += 1;
        }
    }
    if dead > 0 {
        eprintln!(
            "[scan] jsonboards: {dead} of {} boards did not answer",
            boards.len()
        );
    }
    let breakdown = per_vendor
        .iter()
        .map(|(vendor, count)| format!("{vendor} {count}"))
        .collect::<Vec<_>>()
        .join(", ");
    eprintln!(
        "[scan] jsonboards: {} recent engineering postings in Canada/US ({filtered} dropped by the role/region filter, {undated} undated){}",
        out.len(),
        if breakdown.is_empty() {
            String::new()
        } else {
            format!(" [{breakdown}]")
        }
    );
    out
}

async fn fetch_board(
    ctx: &ScanContext,
    board: &Board,
    window: i64,
    timeout: u64,
    max_pages: i64,
    role: Option<Regex>,
) -> (Vec<(Row, Option<i64>)>, bool, usize, usize) {
    let paged = board.ats == "smartrecruiters";
    let pages = if paged { max_pages } else { 1 };
    let mut rows = Vec::new();
    let (mut dropped, mut undated) = (0, 0);
    let mut offset = 0usize;
    for _ in 0..pages {
        let Some(url) = vendor_url(&board.ats, &board.slug, offset) else {
            return (rows, false, dropped, undated);
        };
        let Some(text) = ctx
            .http
            .get_text_with_timeout(&url, &[], Duration::from_secs(timeout), 1)
            .await
        else {
            return (rows, false, dropped, undated);
        };
        let Ok(data) = serde_json::from_str::<Value>(&text) else {
            return (rows, false, dropped, undated);
        };
        let key = list_key(&board.ats);
        let page = match key {
            Some(key) => data.get(key).and_then(Value::as_array),
            None => data.as_array(),
        };
        let Some(page) = page.filter(|page| !page.is_empty()) else {
            return (rows, true, dropped, undated);
        };
        for job in page {
            let row = vendor_row(&board.ats, job, &board.slug);
            if row.title.is_empty() {
                continue;
            }
            let age = parse_age_days("", row.posted);
            if age.is_some_and(|age| age > window) {
                if paged {
                    return (rows, true, dropped, undated);
                }
                continue;
            }
            if role.as_ref().is_some_and(|role| !role.is_match(&row.title)) {
                dropped += 1;
                continue;
            }
            if !region_ok_row(&row) {
                dropped += 1;
                continue;
            }
            if age.is_none() {
                undated += 1;
            }
            rows.push((row, age));
        }
        if page.len() < PAGE_SIZE {
            break;
        }
        offset += PAGE_SIZE;
    }
    (rows, true, dropped, undated)
}

fn vendor_url(vendor: &str, slug: &str, offset: usize) -> Option<String> {
    let slug = percent_encode(slug);
    Some(match vendor {
        "workable" => {
            format!("https://apply.workable.com/api/v1/widget/accounts/{slug}?details=true")
        }
        "smartrecruiters" => format!(
            "https://api.smartrecruiters.com/v1/companies/{slug}/postings?limit=100&offset={offset}"
        ),
        "rippling" => format!("https://ats.rippling.com/api/v1/board/{slug}/jobs"),
        "bamboohr" => format!("https://{slug}.bamboohr.com/careers/list"),
        "recruitee" => format!("https://{slug}.recruitee.com/api/offers/"),
        "breezy" => format!("https://{slug}.breezy.hr/json"),
        "teamtailor" => format!("https://{slug}.teamtailor.com/jobs.json"),
        _ => return None,
    })
}

fn list_key(vendor: &str) -> Option<&'static str> {
    match vendor {
        "workable" => Some("jobs"),
        "smartrecruiters" => Some("content"),
        "bamboohr" => Some("result"),
        "recruitee" => Some("offers"),
        "teamtailor" => Some("items"),
        _ => None,
    }
}

pub const VENDORS: [&str; 7] = [
    "workable",
    "smartrecruiters",
    "rippling",
    "bamboohr",
    "recruitee",
    "breezy",
    "teamtailor",
];

/// Parse one saved board body into postings, without the role/region filters.
/// The live path uses the same extractors, so this is the parser fixture.
pub fn parse_page(vendor: &str, body: &str, slug: &str) -> Vec<Value> {
    let Ok(data) = serde_json::from_str::<Value>(body) else {
        return Vec::new();
    };
    let page = match list_key(vendor) {
        Some(key) => data.get(key).and_then(Value::as_array),
        None => data.as_array(),
    };
    let Some(page) = page else {
        return Vec::new();
    };
    let board = Board {
        name: slug.to_owned(),
        ats: vendor.to_owned(),
        slug: slug.to_owned(),
        tenant: String::new(),
        wd: String::new(),
        site: String::new(),
    };
    let mut out = Vec::new();
    for job in page {
        let row = vendor_row(vendor, job, slug);
        if row.title.is_empty() {
            continue;
        }
        let age = parse_age_days("", row.posted);
        let dedupe = row
            .url
            .as_deref()
            .and_then(apply_signature)
            .map(|(_, _, _, key)| key)
            .or_else(|| {
                row.raw_id
                    .as_ref()
                    .map(|id| format!("ats|{}:{}:{}", board.ats, board.slug.to_lowercase(), id))
            });
        let mut extra = json!({ "board": format!("{}|{}", board.ats, board.slug) });
        if age.is_none() {
            extra["date_missing"] = json!(true);
        }
        let posted = row.posted.map(|date| date.format("%Y-%m-%d").to_string());
        out.push(make_posting(
            "ats",
            &board.name,
            &row.title,
            &row.location,
            row.url.as_deref(),
            posted.as_deref(),
            age,
            None,
            Some(&board.ats),
            row.raw_id.as_deref(),
            Some(&extra),
            dedupe.as_deref(),
        ));
    }
    out
}

fn vendor_row(vendor: &str, job: &Value, slug: &str) -> Row {
    match vendor {
        "workable" => Row {
            title: string(job, "title"),
            location: join(&[
                string_opt(job, "city"),
                string_opt(job, "state"),
                string_opt(job, "country"),
            ]),
            posted: as_date(job.get("published_on")),
            url: string_opt(job, "url").or_else(|| string_opt(job, "application_url")),
            raw_id: string_opt(job, "shortcode").or_else(|| string_opt(job, "code")),
            ..Row::default()
        },
        "smartrecruiters" => {
            let where_ = job.get("location").cloned().unwrap_or(Value::Null);
            let jid = job.get("id").and_then(scalar);
            let code = where_
                .get("country")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            Row {
                title: string(job, "name"),
                location: join(&[
                    where_
                        .get("city")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    where_
                        .get("region")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    (!code.is_empty()).then(|| country_display(&code)),
                ]),
                country_code: (!code.is_empty()).then_some(code),
                posted: as_date(job.get("releasedDate")),
                url: jid
                    .as_ref()
                    .map(|jid| format!("https://jobs.smartrecruiters.com/{slug}/{jid}")),
                raw_id: jid.or_else(|| job.get("uuid").and_then(scalar)),
            }
        }
        "rippling" => Row {
            title: string(job, "name"),
            location: loc_name(job.get("workLocation")),
            posted: None,
            url: string_opt(job, "url"),
            raw_id: string_opt(job, "uuid"),
            ..Row::default()
        },
        "bamboohr" => {
            let where_ = job.get("location").cloned().unwrap_or(Value::Null);
            let jid = job.get("id").and_then(scalar);
            let mut location = join(&[
                where_
                    .get("city")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                where_
                    .get("state")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            ]);
            if location.is_empty() {
                location = loc_name(job.get("atsLocation"));
            }
            Row {
                title: string(job, "jobOpeningName"),
                location,
                posted: None,
                url: jid
                    .as_ref()
                    .map(|jid| format!("https://{slug}.bamboohr.com/careers/{jid}/")),
                raw_id: jid,
                ..Row::default()
            }
        }
        "recruitee" => Row {
            title: string(job, "title"),
            location: join(&[
                string_opt(job, "city"),
                string_opt(job, "state_name"),
                string_opt(job, "country"),
            ]),
            posted: as_date(job.get("published_at")),
            url: string_opt(job, "careers_url").or_else(|| {
                string_opt(job, "slug")
                    .map(|slug_job| format!("https://{slug}.recruitee.com/o/{slug_job}"))
            }),
            raw_id: string_opt(job, "slug").or_else(|| job.get("id").and_then(scalar)),
            ..Row::default()
        },
        "breezy" => {
            let mut location = loc_name(job.get("location"));
            if location.is_empty() {
                location = loc_name(job.get("locations"));
            }
            Row {
                title: string(job, "name"),
                location,
                posted: as_date(job.get("published_date")),
                url: string_opt(job, "url"),
                raw_id: string_opt(job, "id").or_else(|| job.get("id").and_then(scalar)),
                ..Row::default()
            }
        }
        "teamtailor" => {
            let jobposting = job.get("_jobposting").cloned().unwrap_or(Value::Null);
            let mut where_ = jobposting
                .get("jobLocation")
                .cloned()
                .unwrap_or(Value::Null);
            if let Some(items) = where_.as_array() {
                where_ = items.first().cloned().unwrap_or(Value::Null);
            }
            let address = where_.get("address").cloned().unwrap_or(Value::Null);
            let country = address
                .get("addressCountry")
                .map(|value| {
                    value
                        .as_str()
                        .map(country_display)
                        .or_else(|| value.get("name").and_then(Value::as_str).map(str::to_owned))
                        .unwrap_or_default()
                })
                .unwrap_or_default();
            let url = string_opt(job, "url");
            Row {
                title: string(job, "title"),
                location: join(&[
                    address
                        .get("addressLocality")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    address
                        .get("addressRegion")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    (!country.is_empty()).then_some(country),
                ]),
                posted: as_date(job.get("date_published")),
                raw_id: url.as_deref().and_then(|url| url_id(url, r"/jobs/(\d+)")),
                url,
                ..Row::default()
            }
        }
        _ => Row::default(),
    }
}

fn region_ok_row(row: &Row) -> bool {
    if let Some(code) = row.country_code.as_deref() {
        return matches!(
            code.trim().to_lowercase().as_str(),
            "us" | "ca" | "pr" | "gu" | "vi"
        );
    }
    region_ok(&row.location)
}

fn string(job: &Value, key: &str) -> String {
    job.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_owned()
}

fn string_opt(job: &Value, key: &str) -> Option<String> {
    job.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn loc_name(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.trim().to_owned(),
        Some(Value::Object(map)) => map
            .get("name")
            .or_else(|| map.get("label"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_owned(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| loc_name(Some(item)))
            .filter(|item| !item.is_empty())
            .collect::<Vec<_>>()
            .join(", "),
        Some(other) => other.as_str().unwrap_or("").trim().to_owned(),
        None => String::new(),
    }
}

fn join(parts: &[Option<String>]) -> String {
    parts
        .iter()
        .filter_map(|part| part.as_deref())
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

fn as_date(value: Option<&Value>) -> Option<NaiveDate> {
    static DATE: OnceLock<Regex> = OnceLock::new();
    let text = match value {
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
        None => return None,
    };
    let capture = DATE
        .get_or_init(|| Regex::new(r"^(\d{4})-(\d{2})-(\d{2})").unwrap())
        .captures(text.trim())?;
    NaiveDate::from_ymd_opt(
        capture[1].parse().ok()?,
        capture[2].parse().ok()?,
        capture[3].parse().ok()?,
    )
}

fn url_id(url: &str, pattern: &str) -> Option<String> {
    Regex::new(pattern)
        .ok()?
        .captures(url)
        .map(|capture| capture[1].to_owned())
}

fn country_display(code: &str) -> String {
    match code.trim().to_lowercase().as_str() {
        "us" | "pr" | "gu" | "vi" => "United States".into(),
        "ca" => "Canada".into(),
        "gb" => "United Kingdom".into(),
        "ie" => "Ireland".into(),
        "au" => "Australia".into(),
        "nz" => "New Zealand".into(),
        "sg" => "Singapore".into(),
        "in" => "India".into(),
        "de" => "Germany".into(),
        "fr" => "France".into(),
        "es" => "Spain".into(),
        "it" => "Italy".into(),
        "nl" => "Netherlands".into(),
        "se" => "Sweden".into(),
        "no" => "Norway".into(),
        "dk" => "Denmark".into(),
        "fi" => "Finland".into(),
        "pl" => "Poland".into(),
        "pt" => "Portugal".into(),
        "br" => "Brazil".into(),
        "mx" => "Mexico".into(),
        "jp" => "Japan".into(),
        "kr" => "South Korea".into(),
        "cn" => "China".into(),
        "hk" => "Hong Kong".into(),
        "tw" => "Taiwan".into(),
        "il" => "Israel".into(),
        "ae" => "United Arab Emirates".into(),
        "za" => "South Africa".into(),
        other => other.to_uppercase(),
    }
}
