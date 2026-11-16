//! A minimal OpenAI-compatible chat client for the brief generator. The key is
//! read from the environment, the Keychain, or the pool Hermes already keeps.

use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use std::{path::PathBuf, time::Duration};

pub const DEFAULT_BASE: &str = "https://api.deepseek.com/v1";
pub const DEFAULT_MODEL: &str = "deepseek-flash";

#[derive(Debug)]
pub struct LLMError(pub String);

impl std::fmt::Display for LLMError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}
impl std::error::Error for LLMError {}

fn credential() -> Result<(String, String)> {
    let key = std::env::var("DEEPSEEK_API_KEY").ok();
    let base = std::env::var("DEEPSEEK_BASE_URL").ok();
    if let Some(key) = key.filter(|key| !key.is_empty()) {
        return Ok((
            key,
            base.unwrap_or_else(|| DEFAULT_BASE.into())
                .trim_end_matches('/')
                .to_owned(),
        ));
    }
    if let Some(key) = keychain_key() {
        return Ok((
            key,
            base.unwrap_or_else(|| DEFAULT_BASE.into())
                .trim_end_matches('/')
                .to_owned(),
        ));
    }
    let auth = PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .join(".hermes")
        .join("auth.json");
    let text = std::fs::read_to_string(&auth)
        .map_err(|error| LLMError(format!("could not read a DeepSeek credential: {error}")))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|error| LLMError(format!("could not read a DeepSeek credential: {error}")))?;
    let entry = value
        .get("credential_pool")
        .and_then(|pool| pool.get("deepseek"))
        .and_then(Value::as_array)
        .and_then(|pool| pool.first())
        .cloned()
        .unwrap_or(Value::Null);
    let Some(key) = entry.get("access_token").and_then(Value::as_str) else {
        return Err(LLMError(
            "no DeepSeek API key: set DEEPSEEK_API_KEY or add one in Settings".into(),
        )
        .into());
    };
    let base = base
        .or_else(|| {
            entry
                .get("base_url")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| DEFAULT_BASE.into());
    Ok((key.to_owned(), base.trim_end_matches('/').to_owned()))
}

fn keychain_key() -> Option<String> {
    let output = std::process::Command::new("security")
        .args([
            "find-generic-password",
            "-s",
            "com.muhanli.jobhunt.deepseek",
            "-w",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let key = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!key.is_empty()).then_some(key)
}

/// One completion, returning the parsed JSON object from the assistant's
/// content. Retries the transport and JSON-mode flakes, never a 4xx.
pub async fn chat_json(messages: &[Value], attempts: usize) -> Result<Value> {
    let mut last = anyhow!("the model did not return JSON");
    for attempt in 0..attempts {
        match chat_once(messages).await {
            Ok(content) => match parse_json(&content) {
                Ok(value) => return Ok(value),
                Err(error) => last = error,
            },
            Err(error) => {
                last = error;
                tokio::time::sleep(Duration::from_millis(1500 * (attempt as u64 + 1))).await;
            }
        }
    }
    Err(last)
}

async fn chat_once(messages: &[Value]) -> Result<String> {
    let (key, base) = credential()?;
    let model = std::env::var("DEEPSEEK_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.into());
    let payload = json!({
        "model": model,
        "messages": messages,
        "temperature": 0.2,
        "max_tokens": 6000,
        "response_format": {"type": "json_object"},
    });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(180))
        .build()?;
    let mut last = anyhow!("DeepSeek request failed");
    for attempt in 0..3 {
        let response = client
            .post(format!("{base}/chat/completions"))
            .header("Authorization", format!("Bearer {key}"))
            .header("Content-Type", "application/json")
            .body(payload.to_string())
            .send()
            .await;
        match response {
            Ok(response) => {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                if status.is_success() {
                    let value: Value = serde_json::from_str(&body).map_err(|error| {
                        LLMError(format!("unexpected DeepSeek response: {error}"))
                    })?;
                    let content = value
                        .get("choices")
                        .and_then(Value::as_array)
                        .and_then(|choices| choices.first())
                        .and_then(|choice| choice.get("message"))
                        .and_then(|message| message.get("content"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned();
                    return Ok(content);
                }
                if matches!(status.as_u16(), 429 | 500 | 502 | 503 | 504) && attempt < 2 {
                    tokio::time::sleep(Duration::from_secs(2 * (attempt + 1))).await;
                    last = LLMError(format!(
                        "DeepSeek HTTP {status}: {}",
                        body.chars().take(400).collect::<String>()
                    ))
                    .into();
                    continue;
                }
                return Err(LLMError(format!(
                    "DeepSeek HTTP {status}: {}",
                    body.chars().take(400).collect::<String>()
                ))
                .into());
            }
            Err(error) => {
                last = LLMError(format!("DeepSeek request failed: {error}")).into();
                if attempt < 2 {
                    tokio::time::sleep(Duration::from_secs(2 * (attempt + 1))).await;
                }
            }
        }
    }
    Err(last)
}

/// Tolerate a fence, prose, and trailing content: return the first JSON object.
pub fn parse_json(content: &str) -> Result<Value> {
    let mut text = content.trim();
    if text.starts_with("```") {
        let parts: Vec<&str> = text.split("```").collect();
        if parts.len() >= 3 {
            text = parts[1].trim_start();
            if text.to_lowercase().starts_with("json") {
                text = &text[4..];
            }
        }
    }
    let text = text.trim();
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        return object(value);
    }
    let start = text
        .find('{')
        .ok_or_else(|| LLMError("the model did not return JSON".into()))?;
    let mut stream = serde_json::Deserializer::from_str(&text[start..]).into_iter::<Value>();
    match stream.next() {
        Some(Ok(value)) => object(value),
        _ => Err(LLMError("the model returned invalid JSON".into()).into()),
    }
}

fn object(value: Value) -> Result<Value> {
    if value.is_object() {
        Ok(value)
    } else {
        Err(LLMError("the model's JSON was not an object".into()).into())
    }
}
