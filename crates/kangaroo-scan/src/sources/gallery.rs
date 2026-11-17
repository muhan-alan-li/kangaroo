//! startups.gallery: about 20 of the newest postings per run, each one a deep
//! link into the employer's own ATS.

use super::{Source, strip_tags, yaml_str};
use crate::ScanContext;
use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;
use kangaroo_core::parsing::parse_age_days;
use kangaroo_core::posting::{apply_signature, make_posting};
use regex::Regex;
use serde_json::{Value, json};
use serde_yaml::Value as Yaml;
use std::{collections::HashSet, sync::OnceLock, time::Duration};

pub struct Gallery;

#[async_trait]
impl Source for Gallery {
    fn id(&self) -> &'static str {
        "startups_gallery"
    }
    async fn fetch(&self, ctx: &ScanContext, cfg: &Yaml, _filters: &Yaml) -> Result<Vec<Value>> {
        let url = yaml_str(cfg, "url").unwrap_or("https://startups.gallery/jobs");
        let Some(page) = ctx
            .http
            .get_text_with_timeout(url, &[], Duration::from_secs(60), 1)
            .await
        else {
            return Ok(Vec::new());
        };
        let postings = parse(&page);
        for posting in &postings {
            if let (Some(ats), Some(slug)) = (
                posting.get("ats").and_then(Value::as_str),
                posting.get("board_slug").and_then(Value::as_str),
            ) {
                ctx.note_harvest(
                    posting
                        .get("company")
                        .and_then(Value::as_str)
                        .unwrap_or(slug),
                    ats,
                    slug,
                );
            }
        }
        Ok(postings)
    }
}

pub fn parse(page: &str) -> Vec<Value> {
    static ANCHOR: OnceLock<Regex> = OnceLock::new();
    static HEAD: OnceLock<Regex> = OnceLock::new();
    static COMPANY: OnceLock<Regex> = OnceLock::new();
    static LOCATION: OnceLock<Regex> = OnceLock::new();
    static TIME: OnceLock<Regex> = OnceLock::new();
    let anchor = ANCHOR.get_or_init(|| Regex::new(r#"<a[^>]*href="([^"]+)""#).unwrap());
    let head = HEAD.get_or_init(|| Regex::new(r"(?s)<h2[^>]*>(.*?)</h2>").unwrap());
    let company_re = COMPANY.get_or_init(|| {
        Regex::new(r#"(?s)data-framer-name="Company Name"[^>]*>(.*?)</div>"#).unwrap()
    });
    let location_re = LOCATION
        .get_or_init(|| Regex::new(r#"(?s)data-framer-name="Location"[^>]*>(.*?)</div>"#).unwrap());
    let time_re = TIME.get_or_init(|| Regex::new(r#"<time[^>]*datetime="([^"]+)""#).unwrap());

    let needle = r#"<a class="framer-zpdfqp"#;
    let mut out = Vec::new();
    let mut rest = page;
    while let Some(at) = rest.find(needle) {
        let block = &rest[at..];
        let block = block.split("</a>").next().unwrap_or("");
        rest = &rest[at + needle.len()..];

        let Some(capture) = anchor.captures(block) else {
            continue;
        };
        let apply_url = html_escape::decode_html_entities(&capture[1]).into_owned();
        let Some((vendor, slug, job_id, board_key)) = apply_signature(&apply_url) else {
            continue;
        };
        let Some(head) = head.captures(block) else {
            continue;
        };
        let title = strip_tags(&head[1]);
        if title.is_empty() {
            continue;
        }
        let after = &block[head.get(0).map(|m| m.end()).unwrap_or(0)..];

        let company = company_re
            .captures(after)
            .map(|capture| strip_tags(&capture[1]))
            .filter(|company| !company.is_empty())
            .or_else(|| {
                strip_tags(after)
                    .split('·')
                    .map(str::trim)
                    .find(|field| !field.is_empty())
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        if company.is_empty() {
            continue;
        }

        let mut locations = Vec::new();
        let mut seen = HashSet::new();
        for capture in location_re.captures_iter(after) {
            let location = strip_tags(&capture[1]);
            if !location.is_empty() && seen.insert(location.clone()) {
                locations.push(location);
            }
        }
        if locations.is_empty() {
            // Fallback for a card without the named containers: the text after
            // the title is company · location · location · date.
            for field in strip_tags(after).split('·').skip(1) {
                let field = field.trim();
                if !field.is_empty()
                    && !field.starts_with("Nov")
                    && !field.chars().any(|c| c.is_ascii_digit())
                    && seen.insert(field.to_owned())
                {
                    locations.push(field.to_owned());
                }
            }
        }

        let posted = time_re.captures(block).and_then(|capture| {
            let date = capture[1].get(..10)?;
            NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()
        });

        out.push(make_posting(
            "startups_gallery",
            &company,
            &title,
            &locations.join(", "),
            Some(&apply_url),
            posted
                .map(|date| date.format("%Y-%m-%d").to_string())
                .as_deref(),
            parse_age_days("", posted),
            None,
            Some(&vendor),
            Some(&job_id),
            Some(&json!({ "board_slug": slug })),
            Some(&board_key),
        ));
    }
    out
}
