//! What the detail panel shows, as plain values. The layout lives in `main.rs`;
//! everything here is data, so it can be tested without a window.

use kangaroo_protocol::{BriefDetail, PostingDetail, PostingRow};
use serde_json::Value;

/// How much of the posting text the panel draws. A cached page can run to
/// megabytes, and laying out all of it would stall the frame.
pub const TEXT_LIMIT: usize = 6000;

/// One rewrite the brief proposes: the line as the resume reads now, and the
/// line it would become.
pub struct Reword {
    pub requirement: String,
    pub location: String,
    pub after: String,
}

/// The parts of the stage-C payload the panel reads.
pub struct BriefView {
    pub status: String,
    pub generated_at: Option<String>,
    pub error: Option<String>,
    pub verdict: String,
    pub key_skills: Vec<String>,
    pub lead_with: Option<String>,
    pub missing: Vec<String>,
    pub notes: Option<String>,
    pub rewords: Vec<Reword>,
    /// Edits the validator dropped because their `before` was not verbatim.
    pub dropped: usize,
    /// Terms the validator could not find in either the resume or the posting.
    pub flags: Vec<String>,
}

pub fn brief_view(brief: &BriefDetail) -> BriefView {
    let payload = &brief.payload;
    let tailoring = payload.get("tailoring");
    BriefView {
        status: brief.status.clone(),
        generated_at: brief.generated_at.clone(),
        error: brief.error.clone(),
        verdict: text(payload, "verdict").unwrap_or_default(),
        key_skills: strings(payload.get("key_skills")),
        lead_with: tailoring.and_then(|t| text(t, "lead_with")),
        missing: strings(tailoring.and_then(|t| t.get("missing"))),
        notes: tailoring.and_then(|t| text(t, "notes")),
        rewords: rewords(tailoring.and_then(|t| t.get("reword"))),
        dropped: payload
            .get("reword_dropped")
            .and_then(Value::as_array)
            .map_or(0, Vec::len),
        flags: strings(payload.get("reword_flags")),
    }
}

fn rewords(value: Option<&Value>) -> Vec<Reword> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| Reword {
                    requirement: text(item, "req").unwrap_or_default(),
                    location: text(item, "where").unwrap_or_default(),
                    after: text(item, "after").unwrap_or_default(),
                })
                .filter(|reword| !reword.after.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn text(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// The posting text, cut to [`TEXT_LIMIT`] with a marker when it was longer.
pub fn posting_text(detail: &PostingDetail) -> Option<String> {
    let text = detail.description.as_deref()?.trim();
    if text.is_empty() {
        return None;
    }
    if text.chars().count() <= TEXT_LIMIT {
        return Some(text.to_owned());
    }
    let cut: String = text.chars().take(TEXT_LIMIT).collect();
    Some(format!("{cut}\n\n… the rest of the posting is not shown."))
}

/// The label and value pairs the panel lists under the posting text. A pair
/// with no value is left out, so the panel never shows an empty row.
pub fn facts(detail: &PostingDetail) -> Vec<(&'static str, String)> {
    let row = &detail.row;
    let mut facts: Vec<(&'static str, String)> = Vec::new();
    let mut add = |label: &'static str, value: Option<String>| {
        if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
            facts.push((label, value));
        }
    };
    add("Source", Some(row.source.clone()));
    add("Board", row.ats.clone());
    add("Tier", row.tier.clone().map(|tier| tier.replace('_', " ")));
    add("Fit", row.score.map(|score| score.to_string()));
    add("Posted", row.posted.clone());
    add("Expires", row.expires_on.clone());
    add("Salary", detail.salary_raw.clone());
    add("Applied", row.applied_at.clone());
    add("Seen", detail.first_seen_at.clone());
    add("Last seen", detail.last_seen_at.clone());
    add("Keywords", join(&detail.keywords));
    add("Skills held", join(&detail.skills_have));
    add("Skills gap", join(&detail.skills_gap));
    add("Emails", join(&detail.emails));
    let mut flags: Vec<&str> = Vec::new();
    if detail.level_ii {
        flags.push("level II");
    }
    if detail.requires_us_work_auth {
        flags.push("US work authorization");
    }
    if detail.below_comp_floor {
        flags.push("below the pay floor");
    }
    if !flags.is_empty() {
        add("Flags", Some(flags.join(", ")));
    }
    facts
}

fn join(values: &[String]) -> Option<String> {
    (!values.is_empty()).then(|| values.join(", "))
}

/// The one line under the title: company, then location when there is one.
pub fn subtitle(row: &PostingRow) -> String {
    match row
        .location
        .as_deref()
        .filter(|text| !text.trim().is_empty())
    {
        Some(location) => format!("{} · {}", row.company, location),
        None => row.company.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row() -> PostingRow {
        PostingRow {
            id: "0123456789abcdef".into(),
            company: "Fixture Co".into(),
            title: "Junior Software Engineer".into(),
            location: Some("Vancouver, BC".into()),
            url: Some("https://example.com/job".into()),
            source: "ats".into(),
            ats: Some("greenhouse".into()),
            tier: Some("remote_canada".into()),
            score: Some(75),
            age_days: Some(0),
            posted: Some("2026-11-13".into()),
            expires_on: Some("2026-11-20".into()),
            brief_status: None,
            has_text: true,
            applied_at: None,
        }
    }

    fn detail() -> PostingDetail {
        PostingDetail {
            row: row(),
            raw_id: Some("req-1".into()),
            salary_raw: None,
            keywords: vec!["Rust".into()],
            skills_have: vec![],
            skills_gap: vec![],
            emails: vec![],
            level_ii: false,
            requires_us_work_auth: false,
            below_comp_floor: false,
            first_seen_at: Some("2026-11-13T10:00:00Z".into()),
            last_seen_at: Some("2026-11-13T10:00:00Z".into()),
            description: Some("  Build reliable systems.  ".into()),
            description_source: Some("cache".into()),
            brief: None,
        }
    }

    /// The stage-C payload is only a `Value` on the wire, so every field is
    /// optional. A missing one must not panic or draw an empty line.
    #[test]
    fn a_brief_reads_the_stage_c_payload() {
        let brief = BriefDetail {
            status: "ok".into(),
            engine: Some("deepseek".into()),
            generated_at: Some("2026-11-13T12:00:00Z".into()),
            error: None,
            payload: json!({
                "verdict": "Apply: the Rust work matches.",
                "key_skills": ["Rust", "SQLite", " "],
                "tailoring": {
                    "lead_with": "Lead with the engine.",
                    "reword": [
                        {"req": "R1", "where": "Apera, bullet 1", "after": "Built the engine."},
                        {"req": "R2", "where": "no after"}
                    ],
                    "missing": ["No Go."],
                    "notes": "Apply through the board."
                },
                "reword_dropped": [{"req": "R3"}],
                "reword_flags": ["kubernetes"]
            }),
        };
        let view = brief_view(&brief);
        assert_eq!(view.verdict, "Apply: the Rust work matches.");
        assert_eq!(view.key_skills, vec!["Rust".to_string(), "SQLite".into()]);
        assert_eq!(view.lead_with.as_deref(), Some("Lead with the engine."));
        assert_eq!(view.missing, vec!["No Go.".to_string()]);
        assert_eq!(view.notes.as_deref(), Some("Apply through the board."));
        // An edit with no `after` has nothing to show.
        assert_eq!(view.rewords.len(), 1);
        assert_eq!(view.rewords[0].requirement, "R1");
        assert_eq!(view.rewords[0].location, "Apera, bullet 1");
        assert_eq!(view.dropped, 1);
        assert_eq!(view.flags, vec!["kubernetes".to_string()]);
    }

    #[test]
    fn a_failed_brief_keeps_its_error() {
        let view = brief_view(&BriefDetail {
            status: "failed".into(),
            engine: Some("deepseek".into()),
            generated_at: None,
            error: Some("no key".into()),
            payload: json!({"status": "failed"}),
        });
        assert_eq!(view.status, "failed");
        assert_eq!(view.error.as_deref(), Some("no key"));
        assert!(view.verdict.is_empty());
        assert!(view.rewords.is_empty());
    }

    #[test]
    fn the_facts_skip_empty_values() {
        let mut sample = detail();
        sample.keywords.clear();
        sample.row.tier = Some("remote_canada".into());
        let facts = facts(&sample);
        let labels: Vec<&str> = facts.iter().map(|(label, _)| *label).collect();
        assert_eq!(
            labels,
            vec![
                "Source",
                "Board",
                "Tier",
                "Fit",
                "Posted",
                "Expires",
                "Seen",
                "Last seen"
            ]
        );
        let tier = facts.iter().find(|(label, _)| *label == "Tier").unwrap();
        assert_eq!(tier.1, "remote canada");
    }

    #[test]
    fn flags_read_in_words() {
        let mut sample = detail();
        sample.level_ii = true;
        sample.requires_us_work_auth = true;
        let flags = facts(&sample)
            .into_iter()
            .find(|(label, _)| *label == "Flags")
            .map(|(_, value)| value)
            .unwrap();
        assert_eq!(flags, "level II, US work authorization");
    }

    #[test]
    fn long_text_is_cut() {
        let mut sample = detail();
        sample.description = Some("x".repeat(TEXT_LIMIT + 10));
        let text = posting_text(&sample).unwrap();
        assert!(text.ends_with("is not shown."));
        assert!(text.chars().count() < TEXT_LIMIT + 60);
    }

    #[test]
    fn absent_text_is_none() {
        let mut sample = detail();
        sample.description = None;
        assert!(posting_text(&sample).is_none());
        sample.description = Some("   ".into());
        assert!(posting_text(&sample).is_none());
    }

    #[test]
    fn the_subtitle_falls_back_to_the_company() {
        let mut sample = detail();
        assert_eq!(subtitle(&sample.row), "Fixture Co · Vancouver, BC");
        sample.row.location = None;
        assert_eq!(subtitle(&sample.row), "Fixture Co");
    }
}
