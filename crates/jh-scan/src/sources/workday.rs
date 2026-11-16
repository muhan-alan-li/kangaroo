//! Workday boards, read through the board API. Workday is the largest source
//! and throttles by IP, so its worker pool stays deliberately small.

use super::ats::Board;
use super::{yaml_i64, yaml_str, yaml_string_list};
use crate::ScanContext;
use anyhow::Result;
use futures::stream::{self, StreamExt};
use jh_core::filters::region_signals;
use jh_core::posting::make_posting;
use regex::Regex;
use serde_json::{Value, json};
use serde_yaml::Value as Yaml;
use std::{sync::OnceLock, time::Duration};

const FOREIGN_PLACES: &str = r"(?i)\b(australia|austria|bangalore|bengaluru|belgium|bolivia|brazil|bulgaria|cambodia|chile|china|colombia|costa rica|croatia|czech|denmark|dominican republic|ecuador|egypt|el salvador|estonia|finland|france|germany|greece|guatemala|honduras|hong kong|hungary|iceland|india|indonesia|ireland|israel|italy|japan|kazakhstan|kenya|korea|latvia|lithuania|luxembourg|malaysia|mexico|morocco|netherlands|new zealand|nicaragua|nigeria|norway|pakistan|panama|paraguay|peru|philippines|poland|portugal|qatar|romania|russia|saudi|singapore|slovakia|slovenia|south africa|spain|sweden|switzerland|taiwan|thailand|turkey|ukraine|united arab emirates|united kingdom|uruguay|venezuela|vietnam|england|scotland|wales|uk|edinburgh|glasgow|belfast|cardiff|london|dublin|munich|berlin|paris|amsterdam|zurich|geneva|barcelona|madrid|milan|rome|warsaw|krakow|prague|budapest|bucharest|sofia|belgrade|zagreb|hyderabad|mumbai|pune|chennai|gurgaon|noida|kolkata|ahmedabad|kochi|tokyo|osaka|seoul|shanghai|beijing|shenzhen|guangzhou|taipei|tel aviv|haifa|sydney|melbourne|brisbane|perth|adelaide|auckland|wellington|kuala lumpur|jakarta|manila|bangkok|ho chi minh|hanoi|dubai|abu dhabi|istanbul|ankara|cairo|nairobi|lagos|johannesburg|cape town|sao paulo|mexico city|guadalajara|monterrey|bogota|lima|santiago|buenos aires|san jose, costa rica|gurugram|bundesweit|crawley|maidenhead|derrimut)\b";
const FOREIGN_CODES: &str = r"(?i)\b(ind|gbr|aus|deu|fra|sgp|jpn|nld|esp|ita|pol|isr|zaf|phl|mys|tha|vnm|idn|twn|kor|tur|egy|nga|ken|mex|bra|chn|hkg|irl|nzl|dnk|swe|nor|fin|aut|cze|hun|rou|grc|prt|svk|svn|hrv|bgr|ltu|lva|est|srb|ukr)\b";

type BoardResult = (
    Board,
    Vec<(Value, Option<i64>)>,
    bool,
    usize,
    Option<String>,
);

pub fn region_ok(location: &str) -> bool {
    static PLACES: OnceLock<Regex> = OnceLock::new();
    static CODES: OnceLock<Regex> = OnceLock::new();
    if location.trim().is_empty() {
        return true;
    }
    let (ca, us) = region_signals(location);
    if ca || us {
        return true;
    }
    if PLACES
        .get_or_init(|| Regex::new(FOREIGN_PLACES).unwrap())
        .is_match(location)
    {
        return false;
    }
    !CODES
        .get_or_init(|| Regex::new(FOREIGN_CODES).unwrap())
        .is_match(location)
}

pub fn parse_age(text: &str) -> Option<i64> {
    static DAYS: OnceLock<Regex> = OnceLock::new();
    static MONTHS: OnceLock<Regex> = OnceLock::new();
    let text = text.trim().to_lowercase();
    if text.is_empty() {
        return None;
    }
    if text.ends_with("today") {
        return Some(0);
    }
    if text.ends_with("yesterday") {
        return Some(1);
    }
    if let Some(capture) = DAYS
        .get_or_init(|| Regex::new(r"(\d+)\s*\+?\s*day").unwrap())
        .captures(&text)
    {
        return capture[1].parse().ok();
    }
    MONTHS
        .get_or_init(|| Regex::new(r"(\d+)\s*\+?\s*month").unwrap())
        .captures(&text)
        .and_then(|capture| capture[1].parse::<i64>().ok())
        .map(|months| months * 30)
}

pub fn role_regex(words: &[String]) -> Option<Regex> {
    let words: Vec<String> = if words.is_empty() {
        [
            "engineer",
            "engineering",
            "developer",
            "programmer",
            "swe",
            "sde",
        ]
        .iter()
        .map(|word| (*word).to_owned())
        .collect()
    } else {
        words.to_vec()
    };
    let pattern = words
        .iter()
        .map(|word| regex::escape(word))
        .collect::<Vec<_>>()
        .join("|");
    Regex::new(&format!(r"(?i)\b({pattern})\b")).ok()
}

/// Parse one saved Workday board page into `(jobPosting, age_days)` pairs.
/// The live path applies the role and region filters to these same fields.
pub fn parse_posts(body: &str, window: i64) -> Vec<(Value, Option<i64>)> {
    let Ok(data) = serde_json::from_str::<Value>(body) else {
        return Vec::new();
    };
    let posts = data
        .get("jobPostings")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    posts
        .into_iter()
        .filter_map(|post| {
            let age = post
                .get("postedOn")
                .and_then(Value::as_str)
                .and_then(parse_age);
            if age.is_some_and(|age| age > window) {
                return None;
            }
            Some((post, age))
        })
        .collect()
}

pub async fn fetch_boards(ctx: &ScanContext, boards: &[Board], cfg: &Yaml) -> (Vec<Value>, usize) {
    let opts = cfg.get("workday").unwrap_or(&Yaml::Null);
    let page_size = yaml_i64(opts, "page_size").unwrap_or(20).max(1);
    let max_pages = yaml_i64(opts, "max_pages").unwrap_or(2).max(1);
    let workers = yaml_i64(opts, "workers").unwrap_or(16).max(1) as usize;
    let window = yaml_i64(opts, "window_days").unwrap_or(7);
    let timeout = yaml_i64(opts, "timeout").unwrap_or(25).max(1) as u64;
    let search_text = yaml_str(opts, "search_text")
        .unwrap_or("engineer")
        .to_owned();
    let role = role_regex(&yaml_string_list(opts, "title_words"));

    let results: Vec<BoardResult> = stream::iter(boards.iter().cloned().map(|board| {
        let ctx = ctx.clone();
        let role = role.clone();
        let search = search_text.clone();
        async move {
            let (rows, ok, dropped, error) = fetch_board(
                &ctx, &board, page_size, max_pages, window, timeout, role, &search,
            )
            .await;
            (board, rows, ok, dropped, error)
        }
    }))
    .buffer_unordered(workers)
    .collect()
    .await;

    let mut out = Vec::new();
    let mut dead = 0;
    let mut filtered = 0;
    for (board, rows, ok, dropped, error) in results {
        let key = format!("{}|{}|{}", board.tenant, board.wd, board.site);
        ctx.note_board(
            &key,
            Some(board.name.clone()),
            ok,
            rows.len(),
            error.clone(),
        );
        filtered += dropped;
        if !ok {
            dead += 1;
            if let Some(error) = error {
                eprintln!(
                    "[scan] workday: {}/{}/{} failed: {error}",
                    board.tenant, board.wd, board.site
                );
            }
            continue;
        }
        let company = board.name.trim();
        for (post, age) in rows {
            let path = post
                .get("externalPath")
                .and_then(Value::as_str)
                .unwrap_or("");
            if path.is_empty() {
                continue;
            }
            let last = path.trim_end_matches('/').rsplit('/').next().unwrap_or("");
            let url = format!(
                "https://{}.{}.myworkdayjobs.com/en-US/{}{}",
                board.tenant, board.wd, board.site, path
            );
            let extra = json!({
                "board": format!("{}|{}|{}", board.tenant, board.wd, board.site),
                "time_type": post.get("timeType").cloned().unwrap_or(Value::Null),
                "posted_text": post.get("postedOn").cloned().unwrap_or(Value::Null),
            });
            let dedupe = (!last.is_empty()).then(|| format!("ats|{last}"));
            out.push(make_posting(
                "ats",
                company,
                post.get("title").and_then(Value::as_str).unwrap_or(""),
                post.get("locationsText")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                Some(&url),
                None,
                age,
                None,
                Some("workday"),
                (!last.is_empty()).then_some(last),
                Some(&extra),
                dedupe.as_deref(),
            ));
        }
    }
    if dead > 0 {
        eprintln!(
            "[scan] workday: {dead} of {} boards did not answer",
            boards.len()
        );
    }
    eprintln!(
        "[scan] workday: {} recent engineering postings in Canada/US ({filtered} dropped by the role/region filter)",
        out.len()
    );
    (out, dead)
}

#[allow(clippy::too_many_arguments)]
async fn fetch_board(
    ctx: &ScanContext,
    board: &Board,
    page_size: i64,
    max_pages: i64,
    window: i64,
    timeout: u64,
    role: Option<Regex>,
    search_text: &str,
) -> (Vec<(Value, Option<i64>)>, bool, usize, Option<String>) {
    let url = format!(
        "https://{}.{}.myworkdayjobs.com/wday/cxs/{}/{}/jobs",
        board.tenant, board.wd, board.tenant, board.site
    );
    let referer = format!(
        "https://{}.{}.myworkdayjobs.com/{}",
        board.tenant, board.wd, board.site
    );
    let mut rows = Vec::new();
    let mut dropped = 0;
    let mut offset = 0i64;
    for page in 0..max_pages {
        let body = json!({
            "appliedFacets": {},
            "limit": page_size,
            "offset": offset,
            "searchText": search_text,
        })
        .to_string();
        let data = match workday_page(ctx, &url, &body, &referer, timeout).await {
            Ok(data) => data,
            Err(error) => return (rows, false, dropped, Some(error)),
        };
        let posts = data
            .get("jobPostings")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if posts.is_empty() {
            return (rows, true, dropped, None);
        }
        for post in &posts {
            let age = post
                .get("postedOn")
                .and_then(Value::as_str)
                .and_then(parse_age);
            if age.is_some_and(|age| age > window) {
                continue;
            }
            if role.as_ref().is_some_and(|role| {
                !role.is_match(post.get("title").and_then(Value::as_str).unwrap_or(""))
            }) {
                dropped += 1;
                continue;
            }
            if !region_ok(
                post.get("locationsText")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
            ) {
                dropped += 1;
                continue;
            }
            rows.push((post.clone(), age));
        }
        if posts.len() < page_size as usize {
            return (rows, true, dropped, None);
        }
        offset += page_size;
        if page + 1 == max_pages {
            break;
        }
    }
    (rows, true, dropped, None)
}

async fn workday_page(
    ctx: &ScanContext,
    url: &str,
    body: &str,
    referer: &str,
    timeout: u64,
) -> Result<Value, String> {
    let headers = [
        ("Accept", "application/json"),
        ("Content-Type", "application/json"),
        ("Referer", referer),
    ];
    let mut last = "no attempt".to_owned();
    for attempt in 0..3 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(600 * attempt)).await;
        }
        let Some(text) = ctx
            .http
            .post_text_with_timeout(url, body, &headers, Duration::from_secs(timeout), 0)
            .await
        else {
            last = "request failed".to_owned();
            continue;
        };
        match serde_json::from_str::<Value>(&text) {
            Ok(data) => return Ok(data),
            Err(error) => last = error.to_string(),
        }
    }
    Err(last)
}
