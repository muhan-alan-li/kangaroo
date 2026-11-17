use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_yaml::Value;
use std::{collections::BTreeMap, fs::File, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourcesConfig {
    #[serde(default)]
    pub sources: Vec<BTreeMap<String, Value>>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierConfig {
    pub name: Option<String>,
    pub country: Option<String>,
    pub label: Option<String>,
    #[serde(default)]
    pub cities: Vec<String>,
    #[serde(default)]
    pub ambiguous_cities: Vec<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FiltersConfig {
    #[serde(default)]
    pub titles_include: Vec<String>,
    #[serde(default)]
    pub titles_exclude: Vec<String>,
    #[serde(default)]
    pub titles_role_words: Vec<String>,
    #[serde(default)]
    pub titles_tech_words: Vec<String>,
    #[serde(default)]
    pub location_tiers: Vec<TierConfig>,
    #[serde(default)]
    pub max_age_days: Option<i64>,
    #[serde(default)]
    pub comp_floor_cad: Option<i64>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

pub fn load_sources(path: impl AsRef<Path>) -> Result<SourcesConfig> {
    Ok(serde_yaml::from_reader(File::open(path)?)?)
}
pub fn load_filters(path: impl AsRef<Path>) -> Result<FiltersConfig> {
    Ok(serde_yaml::from_reader(File::open(path)?)?)
}
pub fn as_value<T: Serialize>(value: &T) -> Result<Value> {
    Ok(serde_yaml::to_value(value)?)
}
