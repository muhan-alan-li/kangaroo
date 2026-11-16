//! The SimplifyJobs new-grad feed in both forms: the JSON the repo generates
//! (authoritative) and the README derived from it (fallback).

use super::{Source, first_href, parse_array, strip_tags, yaml_i64, yaml_str, yaml_string_list};
use crate::ScanContext;
use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use jh_core::parsing::parse_age_days;
use jh_core::posting::{apply_signature, make_posting};
use regex::Regex;
use serde_json::{Value, json};
use serde_yaml::Value as Yaml;
use std::sync::OnceLock;

pub struct SimplifyJson;
pub struct SimplifyHtml;

#[async_trait]
impl Source for SimplifyJson {
    fn id(&self) -> &'static str {
        "newgrad_json"
    }
    async fn fetch(&self, ctx: &ScanContext, cfg: &Yaml, _filters: &Yaml) -> Result<Vec<Value>> {
        let Some(url) = yaml_str(cfg, "url") else {
            return Ok(Vec::new());
        };
        let raw = ctx
            .http
            .get_text_with_timeout(url, &[], std::time::Duration::from_secs(90), 1)
            .await;
        let Some(raw) = raw else {
            return Ok(Vec::new());
        };
        Ok(parse_json(&raw, cfg))
    }
}

#[async_trait]
impl Source for SimplifyHtml {
    fn id(&self) -> &'static str {
        "newgrad_list"
    }
    async fn fetch(&self, ctx: &ScanContext, cfg: &Yaml, _filters: &Yaml) -> Result<Vec<Value>> {
        let Some(url) = yaml_str(cfg, "url") else {
            return Ok(Vec::new());
        };
        let page = ctx
            .http
            .get_text_with_timeout(url, &[], std::time::Duration::from_secs(45), 2)
            .await;
        Ok(page.map(|page| parse_html(&page)).unwrap_or_default())
    }
}

/// Parse the machine-readable listings file. A truncated response is salvaged
/// at its last complete object so a 13 MB download cut short still contributes.
pub fn parse_json(raw: &str, cfg: &Yaml) -> Vec<Value> {
    let source = yaml_str(cfg, "id").unwrap_or("newgrad_json");
    let window = yaml_i64(cfg, "window_days").unwrap_or(30);
    let excluded: Vec<String> = yaml_string_list(cfg, "exclude_sponsorship")
        .into_iter()
        .map(|value| value.trim().to_lowercase())
        .collect();
    let records = if raw.trim_start().starts_with('{') {
        if let Ok(value) = serde_json::from_str::<Value>(raw) {
            value
                .get("listings")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        } else {
            parse_array(raw, Some("listings"))
        }
    } else {
        parse_array(raw, None)
    };

    let mut out = Vec::new();
    let (mut closed, mut old, mut nosponsor) = (0, 0, 0);
    for record in records {
        let Some(record) = record.as_object() else {
            continue;
        };
        let active = record
            .get("active")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let visible = record
            .get("is_visible")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !(active && visible) {
            closed += 1;
            continue;
        }
        let sponsorship = record
            .get("sponsorship")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_owned();
        if excluded.contains(&sponsorship.to_lowercase()) {
            nosponsor += 1;
            continue;
        }
        let posted = record
            .get("date_posted")
            .and_then(Value::as_i64)
            .filter(|stamp| *stamp > 0)
            .and_then(|stamp| chrono::DateTime::<Utc>::from_timestamp(stamp, 0))
            .map(|at| at.date_naive());
        let age_days = parse_age_days("", posted);
        if age_days.is_some_and(|age| age > window) {
            old += 1;
            continue;
        }
        let company = record
            .get("company_name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        let title = record
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if company.is_empty() || title.is_empty() {
            continue;
        }
        let apply_url = record
            .get("url")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|url| !url.is_empty());
        let dedupe_key = apply_url
            .and_then(apply_signature)
            .map(|(_, _, _, key)| key);
        let locations = record
            .get("locations")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|item| !item.is_empty())
                    .collect::<Vec<_>>()
                    .join(" | ")
            })
            .unwrap_or_default();
        let raw_id = record
            .get("id")
            .and_then(json_scalar)
            .filter(|id| !id.is_empty());
        let posted_text = posted.map(|date| date.format("%Y-%m-%d").to_string());
        let extra = json!({
            "sponsorship": if sponsorship.is_empty() { Value::Null } else { Value::String(sponsorship) },
            "degrees": record.get("degrees").cloned().unwrap_or(json!([])),
            "company_url": record.get("company_url").cloned().unwrap_or(Value::Null),
            "feed": "listings.json",
        });
        out.push(make_posting(
            source,
            company,
            title,
            &locations,
            apply_url,
            posted_text.as_deref(),
            age_days,
            None,
            None,
            raw_id.as_deref(),
            Some(&extra),
            dedupe_key.as_deref(),
        ));
    }
    eprintln!(
        "[scan] simplify_json: {} recent (skipped {} closed, {} past {}d, {} no-sponsorship)",
        out.len(),
        closed,
        old,
        window,
        nosponsor
    );
    out
}

/// Parse the README-derived HTML. Python reads every `<tr>` on the page,
/// including the closed rows inside collapsed `<details>` blocks; the JSON
/// source is what makes that safe, because it carries the live flags.
pub fn parse_html(page: &str) -> Vec<Value> {
    static ROW: OnceLock<Regex> = OnceLock::new();
    static CELL: OnceLock<Regex> = OnceLock::new();
    static LEADING: OnceLock<Regex> = OnceLock::new();
    let rows = ROW.get_or_init(|| Regex::new(r"(?s)<tr>(.*?)</tr>").unwrap());
    let cells = CELL.get_or_init(|| Regex::new(r"(?s)<td[^>]*>(.*?)</td>").unwrap());
    let leading = LEADING.get_or_init(|| Regex::new(r"^[^\w]+").unwrap());
    let mut out = Vec::new();
    for row in rows.captures_iter(page) {
        let row = &row[1];
        let cells: Vec<String> = cells
            .captures_iter(row)
            .map(|cell| cell[1].to_owned())
            .collect();
        if cells.len() < 4 {
            continue;
        }
        let company = strip_tags(&cells[0]);
        let title = strip_tags(&cells[1]);
        let location = strip_tags(&cells[2]);
        let apply_url = first_href(&cells[3]);
        let age_text = cells
            .get(4)
            .map(|cell| strip_tags(cell))
            .unwrap_or_default();
        if company.is_empty() || title.is_empty() {
            continue;
        }
        let company = leading.replace(&company, "").into_owned();
        let raw_id = format!("{company}|{title}|{location}");
        out.push(make_posting(
            "newgrad_list",
            &company,
            &title,
            &location,
            apply_url.as_deref(),
            None,
            parse_age_days(&age_text, None),
            None,
            None,
            Some(&raw_id),
            Some(&json!({ "age_text": age_text })),
            None,
        ));
    }
    out
}

fn json_scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}
