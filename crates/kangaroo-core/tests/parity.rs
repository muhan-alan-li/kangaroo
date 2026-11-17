use kangaroo_core::{filters, parsing, pipeline, posting};
use serde_json::Value;
use serde_yaml::Value as Yaml;
use std::{fs, path::PathBuf};
fn fixture(name: &str) -> Value {
    serde_json::from_str(
        &fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/python")
                .join(name),
        )
        .unwrap(),
    )
    .unwrap()
}
fn config() -> Yaml {
    serde_yaml::from_str(include_str!("../../../data/config/filters.yaml")).unwrap()
}
#[test]
fn posting_ids_match_python() {
    for c in fixture("posting_ids.json").as_array().unwrap() {
        let i = &c["input"];
        let p = posting::make_posting(
            i["source"].as_str().unwrap(),
            i["company"].as_str().unwrap(),
            i["title"].as_str().unwrap(),
            i["location"].as_str().unwrap(),
            i["url"].as_str(),
            i["posted"].as_str(),
            i["age_days"].as_i64(),
            i["salary"].as_str(),
            i["ats"].as_str(),
            i["raw_id"].as_str(),
            i.get("extra"),
            i["dedupe_key"].as_str(),
        );
        assert_eq!(p, c["expected"]);
    }
}
#[test]
fn apply_signatures_match_python() {
    for c in fixture("apply_signatures.json").as_array().unwrap() {
        let got = posting::apply_signature(c["url"].as_str().unwrap())
            .map(|(a, b, d, e)| serde_json::json!([a, b, d, e]))
            .unwrap_or(serde_json::json!([null, null, null, null]));
        assert_eq!(got, c["expected"], "{}", c["url"]);
    }
}
#[test]
fn filter_cases_match_python() {
    let f = config();
    let cases = fixture("filters.json");
    for c in cases["titles"].as_array().unwrap() {
        let (t, r) = filters::title_matches(c["title"].as_str().unwrap(), &f);
        assert_eq!(serde_json::json!([t, r]), c["expected"])
    }
    for c in cases["levels"].as_array().unwrap() {
        assert_eq!(
            filters::is_level_ii(c["title"].as_str().unwrap(), &f),
            c["expected"]
        )
    }
    for c in cases["locations"].as_array().unwrap() {
        let (ok, k, t) = filters::location_matches(c["location"].as_str(), &f);
        assert_eq!(serde_json::json!([ok, k, t]), c["expected"]);
    }
}
#[test]
fn salary_cases_match_python() {
    for c in fixture("salary.json").as_array().unwrap() {
        let got = parsing::parse_salary(c["input"].as_str())
            .map(|(a, b, e)| serde_json::json!([a, b, e]))
            .unwrap_or(Value::Null);
        assert_eq!(got, c["expected"]);
    }
}
#[test]
fn pipeline_cases_match_python() {
    let f = config();
    let c = fixture("pipeline.json");
    let (kept, _) = pipeline::normalize_and_filter(c["postings"].as_array().unwrap().clone(), &f);
    let mut kept = kept;
    kept.sort_by_key(pipeline::sort_key);
    let summary:Vec<_>=kept.iter().map(|p|serde_json::json!({"id":p["id"],"score":p["score"],"tier":p["tier"],"tier_rank":p["tier_rank"]})).collect();
    assert_eq!(summary, c["expected"].as_array().unwrap().to_vec());
    let (capped, dropped) = pipeline::cap_per_tier(kept.clone(), 1);
    let cap_ids: Vec<_> = capped.iter().map(|p| p["id"].clone()).collect();
    assert_eq!(cap_ids, c["cap_ids"].as_array().unwrap().to_vec());
    assert_eq!(dropped as i64, c["cap_dropped"].as_i64().unwrap());
    assert_eq!(
        serde_json::to_value(pipeline::tier_counts(&kept, &f)).unwrap(),
        c["tier_counts"]
    );
}
#[test]
fn collapse_cases_match_python() {
    for c in fixture("collapse.json").as_array().unwrap() {
        let p = posting::collapse_requisitions(c["postings"].as_array().unwrap().clone());
        let ids: Vec<_> = p.iter().map(|x| x["id"].clone()).collect();
        assert_eq!(ids, c["keep_ids"].as_array().unwrap().to_vec());
    }
}
#[test]
fn parsing_helpers_cover_dates_and_age() {
    assert_eq!(
        parsing::parse_date("September 18, 2026")
            .unwrap()
            .to_string(),
        "2026-09-18"
    );
    assert_eq!(
        parsing::parse_date("Sep 28, 2026").unwrap().to_string(),
        "2026-09-28"
    );
    assert_eq!(parsing::parse_age_days("2w", None), Some(14));
    assert_eq!(parsing::parse_age_days("3h", None), Some(0));
}
