//! Job Bank Canada: the official local feed and the only Canadian source
//! that carries a real posted date.

use super::{Source, percent_encode, strip_tags, yaml_str, yaml_string_list};
use crate::ScanContext;
use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;
use futures::stream::{self, StreamExt};
use jh_core::filters::location_matches;
use jh_core::parsing::{parse_age_days, parse_date};
use jh_core::posting::make_posting;
use regex::Regex;
use serde_json::Value;
use serde_yaml::Value as Yaml;
use std::{collections::HashMap, sync::OnceLock, time::Duration};

pub struct JobBank;

#[derive(Debug, Clone, Default)]
pub struct Detail {
    pub title: Option<String>,
    pub description: Option<String>,
    pub deadline: Option<String>,
    pub apply_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Listing {
    pub jid: String,
    pub title: String,
    pub company: String,
    pub location: String,
    pub posted: Option<NaiveDate>,
    pub salary: Option<String>,
    pub partner: bool,
    pub title_generic: Option<String>,
    pub detail: Option<Detail>,
}

#[async_trait]
impl Source for JobBank {
    fn id(&self) -> &'static str {
        "jobbank"
    }
    async fn fetch(&self, ctx: &ScanContext, cfg: &Yaml, filters: &Yaml) -> Result<Vec<Value>> {
        let location = yaml_str(cfg, "location").unwrap_or("Vancouver, BC");
        let queries = yaml_string_list(cfg, "search_strings");
        let queries = if queries.is_empty() {
            vec!["software".to_owned()]
        } else {
            queries
        };

        let mut listings: Vec<Listing> = Vec::new();
        let mut seen = HashMap::<String, usize>::new();
        for query in queries {
            let url = format!(
                "https://www.jobbank.gc.ca/jobsearch/jobsearch?searchstring={}&locationstring={}&sort=M",
                percent_encode(&query),
                percent_encode(location)
            );
            let Some(page) = ctx.http.get_text(&url, &[]).await else {
                continue;
            };
            for listing in parse_search(&page, location) {
                if seen.insert(listing.jid.clone(), listings.len()).is_none() {
                    listings.push(listing);
                }
            }
            tokio::time::sleep(Duration::from_millis(600)).await;
        }

        let mut targets: Vec<usize> = Vec::new();
        for (index, listing) in listings.iter().enumerate() {
            if !listing.partner || targets.len() >= 60 {
                continue;
            }
            let (ok, _, _) = location_matches(Some(&listing.location), filters);
            if ok {
                targets.push(index);
            }
        }

        let fetched = targets.len();
        let results: Vec<(usize, Detail)> = stream::iter(targets.into_iter().map(|index| {
            let ctx = ctx.clone();
            let jid = listings[index].jid.clone();
            async move {
                tokio::time::sleep(Duration::from_millis(200)).await;
                (index, fetch_detail(&ctx, &jid).await)
            }
        }))
        .buffer_unordered(5)
        .collect()
        .await;
        for (index, detail) in results {
            let listing = &mut listings[index];
            if let Some(title) = detail.title.clone() {
                listing.title_generic = Some(listing.title.clone());
                listing.title = title;
            }
            listing.detail = Some(detail);
        }

        eprintln!(
            "[scan] jobbank: {} unique, {} detail fetches for real titles",
            listings.len(),
            fetched
        );

        Ok(listings
            .into_iter()
            .map(|listing| {
                let posted = listing
                    .posted
                    .map(|date| date.format("%Y-%m-%d").to_string());
                let extra = listing_extra(&listing);
                make_posting(
                    "jobbank",
                    &listing.company,
                    &listing.title,
                    &listing.location,
                    Some(&format!(
                        "https://www.jobbank.gc.ca/jobsearch/jobposting/{}",
                        listing.jid
                    )),
                    posted.as_deref(),
                    parse_age_days("", listing.posted),
                    listing.salary.as_deref(),
                    None,
                    Some(&listing.jid),
                    extra.as_ref(),
                    None,
                )
            })
            .collect())
    }
}

pub fn parse_search(page: &str, fallback_location: &str) -> Vec<Listing> {
    static JID: OnceLock<Regex> = OnceLock::new();
    static TITLE: OnceLock<Regex> = OnceLock::new();
    static BUSINESS: OnceLock<Regex> = OnceLock::new();
    static DATE: OnceLock<Regex> = OnceLock::new();
    static SALARY: OnceLock<Regex> = OnceLock::new();
    static LOCATION: OnceLock<Regex> = OnceLock::new();
    let jid = JID.get_or_init(|| Regex::new(r#"id="article-(\d+)""#).unwrap());
    let title = TITLE
        .get_or_init(|| Regex::new(r#"(?s)<span class="noctitle">\s*(.*?)\s*</span>"#).unwrap());
    let business = BUSINESS
        .get_or_init(|| Regex::new(r#"(?s)<li class="business">\s*(.*?)\s*</li>"#).unwrap());
    let date = DATE.get_or_init(|| Regex::new(r#"(?s)<li class="date">\s*(.*?)\s*</li>"#).unwrap());
    let salary =
        SALARY.get_or_init(|| Regex::new(r#"(?s)<li class="salary">\s*(.*?)\s*</li>"#).unwrap());
    let location = LOCATION
        .get_or_init(|| Regex::new(r#"(?s)<li class="location">\s*(.*?)\s*</li>"#).unwrap());

    let mut out = Vec::new();
    for chunk in page.split("<article").skip(1) {
        let chunk = chunk.split("</article>").next().unwrap_or("");
        let Some(jid) = jid.captures(chunk).map(|capture| capture[1].to_owned()) else {
            continue;
        };
        let Some(title) = title.captures(chunk).map(|capture| strip_tags(&capture[1])) else {
            continue;
        };
        let company = business
            .captures(chunk)
            .map(|capture| strip_tags(&capture[1]))
            .filter(|company| !company.is_empty())
            .unwrap_or_else(|| "Unknown".to_owned());
        let posted = date
            .captures(chunk)
            .map(|capture| parse_date(&strip_tags(&capture[1])))
            .unwrap_or(None);
        let location = location
            .captures(chunk)
            .map(|capture| {
                strip_tags(&capture[1])
                    .replace("Location", "")
                    .trim()
                    .to_owned()
            })
            .filter(|location| !location.is_empty())
            .unwrap_or_else(|| fallback_location.to_owned());
        let salary = salary
            .captures(chunk)
            .map(|capture| {
                strip_tags(&capture[1])
                    .replace("Salary", "")
                    .trim()
                    .to_owned()
            })
            .filter(|salary| !salary.is_empty());
        out.push(Listing {
            jid,
            title,
            company,
            location,
            posted,
            salary,
            partner: chunk.contains("class=\"job-source"),
            title_generic: None,
            detail: None,
        });
    }
    out
}

pub fn parse_detail(page: &str) -> Detail {
    static TITLE: OnceLock<Regex> = OnceLock::new();
    static LABEL: OnceLock<Regex> = OnceLock::new();
    static DESCRIPTION: OnceLock<Regex> = OnceLock::new();
    static DEADLINE: OnceLock<Regex> = OnceLock::new();
    static LINK: OnceLock<Regex> = OnceLock::new();
    static HREF: OnceLock<Regex> = OnceLock::new();
    static TRAILING: OnceLock<Regex> = OnceLock::new();
    let mut detail = Detail::default();

    let title = TITLE.get_or_init(|| {
        Regex::new(r#"(?s)<span class="source-title-inner">(.*?)</span>\s*</p>"#).unwrap()
    });
    if let Some(capture) = title.captures(page) {
        let label = LABEL.get_or_init(|| {
            Regex::new(r#"(?s)<span class="orig-title-label">.*?</span>"#).unwrap()
        });
        let inner = label.replace_all(&capture[1], " ");
        let clean = strip_tags(&inner);
        detail.title = (!clean.is_empty()).then_some(clean);
    }

    let description = DESCRIPTION
        .get_or_init(|| Regex::new(r#"(?s)property="description">(.*?)</span>"#).unwrap());
    if let Some(capture) = description.captures(page) {
        // The page escapes the employer's HTML a second time, so unescape
        // before stripping tags. The saved body may also end mid-tag.
        let text = strip_tags(&html_escape::decode_html_entities(&capture[1]));
        let trailing = TRAILING.get_or_init(|| Regex::new(r"<[^>]*$").unwrap());
        let text = trailing.replace(&text, "");
        let text: String = text.trim().chars().take(6000).collect();
        detail.description = (!text.trim().is_empty()).then_some(text.trim().to_owned());
    }

    let deadline = DEADLINE.get_or_init(|| {
        Regex::new(r#"property="validThrough">\s*([0-9]{4}-[0-9]{2}-[0-9]{2})"#).unwrap()
    });
    if let Some(capture) = deadline.captures(page) {
        detail.deadline = Some(capture[1].to_owned());
    }

    let tag = LINK.get_or_init(|| Regex::new(r#"<a[^>]*id="externalJobLink"[^>]*>"#).unwrap());
    if let Some(tag) = tag.find(page) {
        let href = HREF.get_or_init(|| Regex::new(r#"href="([^"]+)""#).unwrap());
        if let Some(capture) = href.captures(tag.as_str()) {
            detail.apply_url = Some(html_escape::decode_html_entities(&capture[1]).into_owned());
        }
    }
    detail
}

async fn fetch_detail(ctx: &ScanContext, jid: &str) -> Detail {
    let url = format!("https://www.jobbank.gc.ca/jobsearch/jobposting/{jid}");
    ctx.http
        .get_text_with_timeout(&url, &[], Duration::from_secs(25), 1)
        .await
        .map(|page| parse_detail(&page))
        .unwrap_or_default()
}

fn listing_extra(listing: &Listing) -> Option<Value> {
    let detail = listing.detail.as_ref();
    let mut extra = serde_json::Map::new();
    if let Some(title) = &listing.title_generic {
        extra.insert("title_generic".into(), Value::String(title.clone()));
    }
    if let Some(value) = detail.and_then(|detail| detail.description.clone()) {
        extra.insert("description".into(), Value::String(value));
    }
    if let Some(value) = detail.and_then(|detail| detail.deadline.clone()) {
        extra.insert("deadline".into(), Value::String(value));
    }
    if let Some(value) = detail.and_then(|detail| detail.apply_url.clone()) {
        extra.insert("apply_url".into(), Value::String(value));
    }
    (!extra.is_empty()).then_some(Value::Object(extra))
}
