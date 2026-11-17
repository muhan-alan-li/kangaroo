//! LinkedIn's public guest search. Read only, never signed in; applying there
//! stays manual.

use super::{Source, strip_tags, yaml_str, yaml_string_list};
use crate::ScanContext;
use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;
use kangaroo_core::parsing::parse_age_days;
use kangaroo_core::posting::make_posting;
use regex::Regex;
use serde_json::Value;
use serde_yaml::Value as Yaml;
use std::{sync::OnceLock, time::Duration};

pub struct LinkedIn;

#[async_trait]
impl Source for LinkedIn {
    fn id(&self) -> &'static str {
        "linkedin"
    }
    async fn fetch(&self, ctx: &ScanContext, cfg: &Yaml, _filters: &Yaml) -> Result<Vec<Value>> {
        let base = yaml_str(cfg, "url")
            .unwrap_or("https://www.linkedin.com/jobs-guest/jobs/api/seeMoreJobPostings/search");
        let recency = yaml_str(cfg, "recency").unwrap_or("");
        let keywords = yaml_string_list(cfg, "keywords");
        let locations = yaml_string_list(cfg, "locations");
        let mut out = Vec::new();
        for location in &locations {
            for keyword in &keywords {
                let query = form_encode(&[
                    ("keywords", keyword),
                    ("location", location),
                    ("start", "0"),
                    ("f_TPR", recency),
                ]);
                let url = format!("{base}?{query}");
                if let Some(page) = ctx
                    .http
                    .get_text_with_timeout(&url, &[], Duration::from_secs(30), 1)
                    .await
                {
                    out.extend(parse(&page, location));
                }
                tokio::time::sleep(Duration::from_millis(1200)).await;
            }
        }
        Ok(out)
    }
}

pub fn parse(page: &str, fallback_location: &str) -> Vec<Value> {
    static URN: OnceLock<Regex> = OnceLock::new();
    static TITLE: OnceLock<Regex> = OnceLock::new();
    static COMPANY: OnceLock<Regex> = OnceLock::new();
    static WHERE: OnceLock<Regex> = OnceLock::new();
    static WHEN: OnceLock<Regex> = OnceLock::new();
    let urn =
        URN.get_or_init(|| Regex::new(r#"data-entity-urn="urn:li:jobPosting:(\d+)""#).unwrap());
    let title = TITLE.get_or_init(|| {
        Regex::new(r#"(?s)class="base-search-card__title[^"]*">\s*(.*?)\s*</h3>"#).unwrap()
    });
    let company = COMPANY.get_or_init(|| {
        Regex::new(r#"(?s)class="base-search-card__subtitle[^"]*">.*?>\s*(.*?)\s*</a>"#).unwrap()
    });
    let where_ = WHERE.get_or_init(|| {
        Regex::new(r#"(?s)class="job-search-card__location">\s*(.*?)\s*</span>"#).unwrap()
    });
    let when = WHEN.get_or_init(|| {
        Regex::new(r#"class="job-search-card__listdate[^"]*"\s+datetime="([\d-]+)""#).unwrap()
    });

    let mut out = Vec::new();
    let mut rest = page;
    while let Some(at) = rest.find("<li") {
        let card = &rest[at..];
        rest = &rest[at + 3..];
        let Some(urn) = urn.captures(card) else {
            continue;
        };
        let Some(title_match) = title.captures(card) else {
            continue;
        };
        let posted = when
            .captures(card)
            .and_then(|capture| NaiveDate::parse_from_str(&capture[1], "%Y-%m-%d").ok());
        let company = company
            .captures(card)
            .map(|capture| strip_tags(&capture[1]))
            .filter(|company| !company.is_empty())
            .unwrap_or_else(|| "Unknown".to_owned());
        let title = strip_tags(&title_match[1]);
        let location = where_
            .captures(card)
            .map(|capture| strip_tags(&capture[1]))
            .filter(|location| !location.is_empty())
            .unwrap_or_else(|| fallback_location.to_owned());
        out.push(make_posting(
            "linkedin",
            &company,
            &title,
            &location,
            Some(&format!("https://www.linkedin.com/jobs/view/{}/", &urn[1])),
            posted
                .map(|date| date.format("%Y-%m-%d").to_string())
                .as_deref(),
            parse_age_days("", posted),
            None,
            None,
            Some(&urn[1]),
            None,
            None,
        ));
    }
    out
}

fn form_encode(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                super::percent_encode(key),
                super::percent_encode(value).replace("%20", "+")
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}
