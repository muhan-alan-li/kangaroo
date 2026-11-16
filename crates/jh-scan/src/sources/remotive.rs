//! Remotive's global remote board. Off by default in `sources.yaml`.

use super::{Source, yaml_str};
use crate::ScanContext;
use anyhow::Result;
use async_trait::async_trait;
use jh_core::posting::make_posting;
use serde_json::Value;
use serde_yaml::Value as Yaml;

pub struct Remotive;

#[async_trait]
impl Source for Remotive {
    fn id(&self) -> &'static str {
        "remotive"
    }
    async fn fetch(&self, ctx: &ScanContext, cfg: &Yaml, _filters: &Yaml) -> Result<Vec<Value>> {
        let Some(url) = yaml_str(cfg, "url") else {
            return Ok(Vec::new());
        };
        let Some(body) = ctx.http.get_text(url, &[]).await else {
            return Ok(Vec::new());
        };
        let Ok(value) = serde_json::from_str::<Value>(&body) else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for job in value
            .get("jobs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let raw_id = match job.get("id") {
                Some(Value::String(text)) => Some(text.clone()),
                Some(Value::Number(number)) => Some(number.to_string()),
                _ => None,
            };
            out.push(make_posting(
                "remotive",
                job.get("company_name")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                job.get("title").and_then(Value::as_str).unwrap_or(""),
                job.get("candidate_required_location")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                job.get("url").and_then(Value::as_str),
                None,
                None,
                None,
                None,
                raw_id.as_deref(),
                None,
                None,
            ));
        }
        Ok(out)
    }
}
