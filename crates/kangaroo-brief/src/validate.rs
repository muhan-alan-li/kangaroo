//! Mechanical factuality checks on the brief's `tailoring.reword` edits.
//! `before` must be verbatim in the resume; a notable new term in `after` is
//! flagged, never silently kept.

use regex::Regex;
use serde_json::{Value, json};
use std::sync::OnceLock;

fn normalize(text: &str) -> String {
    static SPACES: OnceLock<Regex> = OnceLock::new();
    let text = text
        .replace(['\u{200b}', '\u{feff}'], "")
        .replace(['\u{2013}', '\u{2014}'], "-");
    SPACES
        .get_or_init(|| Regex::new(r"\s+").unwrap())
        .replace_all(text.trim(), " ")
        .to_lowercase()
}

fn loose(text: &str) -> String {
    static PUNCT: OnceLock<Regex> = OnceLock::new();
    let normalized = normalize(text);
    let no_punct = PUNCT
        .get_or_init(|| Regex::new(r"[^\w\s+#/.\-]").unwrap())
        .replace_all(&normalized, " ");
    normalize(&no_punct)
}

pub fn before_in_resume(before: &str, resume: &str) -> bool {
    let needle = normalize(before);
    if needle.is_empty() {
        return false;
    }
    let hay = normalize(resume);
    hay.contains(&needle) || loose(resume).contains(&loose(before))
}

pub fn invented_terms(after: &str, resume: &str, posting: &str) -> Vec<String> {
    let corpus = format!("{}\n{}", normalize(resume), normalize(posting));
    notable(after)
        .into_iter()
        .filter(|term| !corpus.contains(&normalize(term)))
        .collect()
}

pub fn check_rewords(
    rewords: &[Value],
    resume: &str,
    posting: &str,
) -> (Vec<Value>, Vec<Value>, Vec<String>) {
    let mut kept = Vec::new();
    let mut dropped = Vec::new();
    let mut flags = Vec::new();
    for item in rewords {
        let before = item.get("before").and_then(Value::as_str).unwrap_or("");
        let after = item.get("after").and_then(Value::as_str).unwrap_or("");
        let where_ = item
            .get("where")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if before.is_empty() && after.is_empty() {
            continue;
        }
        if !before_in_resume(before, resume) {
            dropped.push(json!({"where": where_, "reason": "before is not in the resume"}));
            continue;
        }
        let bad = invented_terms(after, resume, posting);
        if !bad.is_empty() {
            flags.push(format!(
                "{}: after names {} (not in resume or posting)",
                if where_.is_empty() { "reword" } else { where_ },
                bad.join(", ")
            ));
        }
        let mut item = item.as_object().cloned().unwrap_or_default();
        item.insert("validated".into(), Value::Bool(true));
        kept.push(Value::Object(item));
    }
    (kept, dropped, flags)
}

fn notable(content: &str) -> Vec<String> {
    static TOKEN: OnceLock<Regex> = OnceLock::new();
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for token in TOKEN
        .get_or_init(|| Regex::new(r"[A-Za-z][A-Za-z0-9.+#]*").unwrap())
        .find_iter(content)
    {
        let token = token.as_str();
        let low = token.trim_matches(|c| c == '.' || c == '#').to_lowercase();
        if low.chars().count() < 2 || !seen.insert(low) {
            continue;
        }
        let first_word = out.is_empty();
        let upper = token.chars().any(char::is_alphabetic)
            && token
                .chars()
                .filter(|c| c.is_alphabetic())
                .all(|c| c.is_uppercase());
        let has_digit = token.chars().any(|c| c.is_ascii_digit());
        let capitalized = token.chars().next().is_some_and(char::is_uppercase) && !first_word;
        if upper || has_digit || capitalized {
            out.push(token.to_owned());
        }
    }
    out
}
