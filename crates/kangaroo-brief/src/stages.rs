//! The three prompts and their model calls. Each stage receives only what it
//! needs: A never sees the resume, B sees the resume before the requirements,
//! C sees the matrix.

use crate::llm;
use anyhow::Result;
use serde_json::{Value, json};
use std::path::Path;

pub const MAX_POSTING_TEXT: usize = 12000;

fn read_prompt(root: &Path, name: &str) -> String {
    std::fs::read_to_string(root.join("data/config").join(name)).unwrap_or_default()
}

fn cap_text(text: &str) -> String {
    let text = if text.trim().is_empty() {
        "(no posting text was available; say so in notes rather than guessing)"
    } else {
        text
    };
    text.chars().take(MAX_POSTING_TEXT).collect()
}

pub async fn requirements(root: &Path, row: &Value, text: &str) -> Result<Value> {
    let system = read_prompt(root, "brief_stage_a_system.md");
    let user = format!(
        "# The posting\n\n- title: {}\n- company: {}\n- location: {}\n\n## Posting text\n\n```\n{}\n```",
        row.get("title").and_then(Value::as_str).unwrap_or(""),
        row.get("company").and_then(Value::as_str).unwrap_or(""),
        row.get("location").and_then(Value::as_str).unwrap_or(""),
        cap_text(text)
    );
    llm::chat_json(
        &[
            json!({"role": "system", "content": system}),
            json!({"role": "user", "content": user}),
        ],
        3,
    )
    .await
}

pub async fn matches(
    root: &Path,
    _row: &Value,
    _text: &str,
    requirements: &Value,
    resume: &str,
) -> Result<Value> {
    let system = read_prompt(root, "brief_stage_b_system.md");
    let user = format!(
        "# Muhan's resume (the only record of what he actually did)\n\n{}\n\n# The requirement contract extracted from this posting\n\n```json\n{}\n```\n\nFor every requirement group in that contract, produce one match entry. Do not skip any.",
        if resume.trim().is_empty() {
            "(unavailable)"
        } else {
            resume
        },
        serde_json::to_string_pretty(requirements)?
    );
    llm::chat_json(
        &[
            json!({"role": "system", "content": system}),
            json!({"role": "user", "content": user}),
        ],
        3,
    )
    .await
}

pub async fn brief(
    root: &Path,
    row: &Value,
    text: &str,
    _requirements: &Value,
    matches: &Value,
    resume: &str,
) -> Result<Value> {
    let system = read_prompt(root, "brief_stage_c_system.md");
    let user = format!(
        "# Muhan's resume (the only record of what he actually did)\n\n{}\n\n# The match matrix\n\n```json\n{}\n```\n\n# The posting, for the verdict's context\n\n- title: {}\n- company: {}\n- location: {}\n- url: {}\n\n## Posting text\n\n```\n{}\n```\n\nWrite the brief. Remember: edits ranked by how much they change the match, at most 3, `before` verbatim from the resume, and no skill invented.",
        if resume.trim().is_empty() {
            "(unavailable)"
        } else {
            resume
        },
        serde_json::to_string_pretty(matches)?,
        row.get("title").and_then(Value::as_str).unwrap_or(""),
        row.get("company").and_then(Value::as_str).unwrap_or(""),
        row.get("location").and_then(Value::as_str).unwrap_or(""),
        row.get("url").and_then(Value::as_str).unwrap_or(""),
        cap_text(text)
    );
    llm::chat_json(
        &[
            json!({"role": "system", "content": system}),
            json!({"role": "user", "content": user}),
        ],
        3,
    )
    .await
}
