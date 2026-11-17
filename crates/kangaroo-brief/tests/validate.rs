//! The validator's contract, ported from `server/brief/validate.py`.

use kangaroo_brief::validate::{before_in_resume, check_rewords, invented_terms};
use serde_json::json;

const RESUME: &str = "# Experience\n\nApera AI, Software Engineer\n- Built a Python ingestion pipeline that cut manual review by 40%.\n- Migrated the lead-scoring service to Postgres.\n";

#[test]
fn verbatim_before_is_kept() {
    let rewords = vec![json!({
        "where": "Experience, Apera AI, first bullet",
        "before": "Built a Python ingestion pipeline that cut manual review by 40%.",
        "after": "Built a Python ingestion pipeline that cut manual review by 40% and fed the scoring service.",
    })];
    let (kept, dropped, _) = check_rewords(&rewords, RESUME, "Python and Postgres");
    assert_eq!(kept.len(), 1);
    assert!(dropped.is_empty());
    assert_eq!(kept[0].get("validated"), Some(&json!(true)));
}

#[test]
fn paraphrase_before_is_dropped() {
    let rewords = vec![json!({
        "where": "Experience, Apera AI, first bullet",
        "before": "Built a Python pipeline that reduced manual review by 40 percent.",
        "after": "Built a Python pipeline.",
    })];
    let (kept, dropped, _) = check_rewords(&rewords, RESUME, "");
    assert!(kept.is_empty());
    assert_eq!(dropped.len(), 1);
    assert_eq!(
        dropped[0].get("reason").and_then(|value| value.as_str()),
        Some("before is not in the resume")
    );
}

#[test]
fn line_wrapped_before_still_matches() {
    assert!(before_in_resume(
        "Built a Python ingestion pipeline that cut manual review by 40",
        "Built a Python ingestion pipeline that\ncut manual review by 40%."
    ));
}

#[test]
fn an_invented_term_is_flagged_but_kept() {
    assert_eq!(
        invented_terms("Built a Rust ingestion service.", RESUME, "Python"),
        Vec::<String>::new()
    );
    assert_eq!(
        invented_terms(
            "Built a Python ingestion service on AWS today",
            RESUME,
            "Python"
        ),
        vec!["AWS"]
    );
    let rewords = vec![json!({
        "where": "Experience, Apera AI, first bullet",
        "before": "Built a Python ingestion pipeline that cut manual review by 40%.",
        "after": "Built a Python ingestion pipeline on AWS today that cut manual review by 40%.",
    })];
    let (kept, dropped, flags) = check_rewords(&rewords, RESUME, "Python");
    assert_eq!(kept.len(), 1);
    assert!(dropped.is_empty());
    assert_eq!(flags.len(), 1);
    assert!(flags[0].contains("AWS"));
}

#[test]
fn a_term_from_the_posting_is_not_flagged() {
    let rewords = vec![json!({
        "where": "Experience",
        "before": "Migrated the lead-scoring service to Postgres.",
        "after": "Migrated the lead-scoring service to Postgres and Kafka.",
    })];
    let (kept, _dropped, flags) = check_rewords(&rewords, RESUME, "Kafka");
    assert_eq!(kept.len(), 1);
    assert!(flags.is_empty());
}
