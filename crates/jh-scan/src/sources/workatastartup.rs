//! Work at a Startup (Y Combinator): the anonymous job feed, read out of the
//! Inertia page payload.

use super::{Source, yaml_string_list};
use crate::ScanContext;
use anyhow::Result;
use async_trait::async_trait;
use jh_core::posting::make_posting;
use regex::Regex;
use serde_json::{Value, json};
use serde_yaml::Value as Yaml;
use std::{collections::HashSet, sync::OnceLock, time::Duration};

const BASE: &str = "https://www.workatastartup.com";
const ACCEPT: &str = "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8";

const DEFAULT_PATHS: [&str; 12] = [
    "/jobs",
    "/companies?role=eng",
    "/jobs/l/software-engineer",
    "/jobs/l/designer",
    "/jobs/l/recruiting",
    "/jobs/l/science",
    "/jobs/l/product-manager",
    "/jobs/l/operations",
    "/jobs/l/sales-manager",
    "/jobs/l/marketing",
    "/jobs/l/legal",
    "/jobs/l/finance",
];

pub struct WorkAtAStartup;

#[async_trait]
impl Source for WorkAtAStartup {
    fn id(&self) -> &'static str {
        "workatastartup"
    }
    async fn fetch(&self, ctx: &ScanContext, cfg: &Yaml, _filters: &Yaml) -> Result<Vec<Value>> {
        let mut paths = yaml_string_list(cfg, "paths");
        if paths.is_empty() {
            paths = DEFAULT_PATHS
                .iter()
                .map(|path| (*path).to_owned())
                .collect();
        }
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut dead = 0;
        for path in &paths {
            let url = if path.starts_with('/') {
                format!("{BASE}{path}")
            } else {
                path.clone()
            };
            let Some(page) = ctx
                .http
                .get_text_with_timeout(&url, &[("Accept", ACCEPT)], Duration::from_secs(45), 1)
                .await
            else {
                dead += 1;
                continue;
            };
            let Some(payload) = payload(&page) else {
                dead += 1;
                continue;
            };
            let jobs = payload
                .get("props")
                .and_then(|props| props.get("jobs"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for job in jobs {
                let Some(jid) = job.get("id").and_then(scalar) else {
                    continue;
                };
                if !seen.insert(jid.clone()) {
                    continue;
                }
                let title = job
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                let company = job
                    .get("companyName")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                if title.is_empty() || company.is_empty() {
                    continue;
                }
                let extra = json!({
                    "date_missing": true,
                    "company_slug": job.get("companySlug").cloned().unwrap_or(Value::Null),
                    "batch": job.get("companyBatch").cloned().unwrap_or(Value::Null),
                    "role_type": job.get("roleType").cloned().unwrap_or(Value::Null),
                    "job_type": job.get("jobType").cloned().unwrap_or(Value::Null),
                });
                out.push(make_posting(
                    "workatastartup",
                    company,
                    title,
                    job.get("location").and_then(Value::as_str).unwrap_or(""),
                    Some(&format!("{BASE}/jobs/{jid}")),
                    None,
                    None,
                    job.get("salary").and_then(Value::as_str),
                    None,
                    Some(&jid),
                    Some(&extra),
                    Some(&format!("workatastartup|{jid}")),
                ));
            }
        }
        eprintln!(
            "[scan] workatastartup: {} distinct postings from {} of {} feed pages ({} did not answer; no publish date exists on this site, so all are tagged date_missing)",
            out.len(),
            paths.len() - dead,
            paths.len(),
            dead
        );
        Ok(out)
    }
}

pub fn payload(page: &str) -> Option<Value> {
    static PAGE: OnceLock<Regex> = OnceLock::new();
    let capture = PAGE
        .get_or_init(|| Regex::new(r#"data-page="([^"]+)""#).unwrap())
        .captures(page)?;
    let decoded = html_escape::decode_html_entities(&capture[1]);
    serde_json::from_str(&decoded).ok()
}

fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}
