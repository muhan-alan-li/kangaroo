//! One module per feed, behind one trait. A source does network work only: it
//! returns plain `Value` postings and writes no state.

pub mod ats;
pub mod gallery;
pub mod jobbank;
pub mod jsonboards;
pub mod linkedin;
pub mod remotive;
pub mod simplify;
pub mod workatastartup;
pub mod workday;

use crate::ScanContext;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use serde_yaml::Value as Yaml;

#[async_trait]
pub trait Source: Send + Sync {
    fn id(&self) -> &'static str;
    async fn fetch(&self, ctx: &ScanContext, cfg: &Yaml, filters: &Yaml) -> Result<Vec<Value>>;
}

/// Build the source named by `sources.yaml`'s `kind`.
pub fn build(kind: &str) -> Option<Box<dyn Source>> {
    match kind {
        "simplify" => Some(Box::new(simplify::SimplifyHtml)),
        "simplify_json" => Some(Box::new(simplify::SimplifyJson)),
        "jobbank" => Some(Box::new(jobbank::JobBank)),
        "ats" => Some(Box::new(ats::AtsBoards)),
        "startups_gallery" => Some(Box::new(gallery::Gallery)),
        "workatastartup" => Some(Box::new(workatastartup::WorkAtAStartup)),
        "linkedin" => Some(Box::new(linkedin::LinkedIn)),
        "remotive" => Some(Box::new(remotive::Remotive)),
        _ => None,
    }
}

pub(crate) fn yaml_str<'a>(value: &'a Yaml, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Yaml::as_str)
}

pub(crate) fn yaml_i64(value: &Yaml, key: &str) -> Option<i64> {
    value.get(key).and_then(Yaml::as_i64)
}

pub(crate) fn yaml_seq<'a>(value: &'a Yaml, key: &str) -> &'a [Yaml] {
    value
        .get(key)
        .and_then(Yaml::as_sequence)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

pub(crate) fn yaml_string_list(value: &Yaml, key: &str) -> Vec<String> {
    yaml_seq(value, key)
        .iter()
        .filter_map(Yaml::as_str)
        .map(str::to_owned)
        .collect()
}

/// `urllib.parse.quote` for the characters Job Bank queries use: everything
/// except unreserved characters and `/` survives unescaped.
pub(crate) fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Python `core.strip_tags`: remove tags, decode entities, collapse spaces.
pub fn strip_tags(fragment: &str) -> String {
    use regex::Regex;
    use std::sync::OnceLock;
    static TAGS: OnceLock<Regex> = OnceLock::new();
    static SPACES: OnceLock<Regex> = OnceLock::new();
    let no_tags = TAGS
        .get_or_init(|| Regex::new(r"(?s)<[^>]*>").unwrap())
        .replace_all(fragment, " ");
    let decoded = html_escape::decode_html_entities(&no_tags);
    SPACES
        .get_or_init(|| Regex::new(r"\s+").unwrap())
        .replace_all(decoded.trim(), " ")
        .into_owned()
}

/// The first non-skipped `href` in a fragment, matching Python `first_href`.
pub(crate) fn first_href(fragment: &str) -> Option<String> {
    use regex::Regex;
    use std::sync::OnceLock;
    static HREF: OnceLock<Regex> = OnceLock::new();
    let re = HREF.get_or_init(|| Regex::new(r#"href=["']([^"']+)["']"#).unwrap());
    for capture in re.captures_iter(fragment) {
        let href = &capture[1];
        if ["imgur.com", "simplify.jobs/c/", "github.com"]
            .iter()
            .any(|skip| href.contains(skip))
        {
            continue;
        }
        return Some(html_escape::decode_html_entities(href).into_owned());
    }
    None
}

/// Complete JSON objects at the top level of an array whose body starts at
/// `start`. The saved parse fixtures are 4 MiB prefixes cut mid-token; a live
/// response parses normally, but this salvage keeps a truncated response
/// useful instead of throwing the whole board away.
pub(crate) fn array_objects(raw: &str, start: usize) -> Vec<&str> {
    let bytes = raw.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut begin = None;
    let mut in_string = false;
    let mut escaped = false;
    let mut index = start;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else {
            match byte {
                b'"' => in_string = true,
                b'{' => {
                    if depth == 0 {
                        begin = Some(index);
                    }
                    depth += 1;
                }
                b'}' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0
                        && let Some(start) = begin.take()
                    {
                        out.push(&raw[start..=index]);
                    }
                }
                _ => {}
            }
        }
        index += 1;
    }
    out
}

/// Parse a full JSON body, falling back to the complete array elements in a
/// truncated one. `marker` locates the array inside an object (`"jobs"`), or is
/// `None` when the body is the array itself.
pub(crate) fn parse_array(raw: &str, marker: Option<&str>) -> Vec<Value> {
    let parsed = if let Some(marker) = marker {
        let start = raw
            .find(marker)
            .and_then(|at| raw[at..].find('[').map(|i| at + i));
        if let Ok(value) = serde_json::from_str::<Value>(raw) {
            return value
                .get(marker)
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
        }
        start
    } else {
        if let Ok(value) = serde_json::from_str::<Value>(raw) {
            return value.as_array().cloned().unwrap_or_default();
        }
        raw.find('[')
    };
    let Some(start) = parsed else {
        return Vec::new();
    };
    array_objects(raw, start + 1)
        .into_iter()
        .filter_map(|object| serde_json::from_str(object).ok())
        .collect()
}
