//! Parser tests against the saved source responses in `fixtures/http/`.
//! These are the S4 acceptance tests: each parser must match what Python sees
//! in the same body.

use jh_scan::sources::{ats, gallery, jobbank, jsonboards, simplify, workatastartup, workday};
use serde_json::Value;

fn fixture(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/http")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()))
}

fn assert_posting_id(value: &Value) {
    let id = value.get("id").and_then(Value::as_str).expect("an id");
    assert_eq!(id.len(), 16, "id {id} is not 16 characters");
    assert!(
        id.chars()
            .all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase()),
        "id {id} is not lowercase hex"
    );
}

#[test]
fn simplify_json_reads_live_records() {
    let body = fixture("simplify_json/listings.json");
    let cfg: serde_yaml::Value = serde_yaml::from_str(
        "id: newgrad_json\nwindow_days: 400\nexclude_sponsorship: [\"Does Not Offer Sponsorship\"]\n",
    )
    .unwrap();
    let postings = simplify::parse_json(&body, &cfg);
    assert!(postings.len() > 50, "only {} postings", postings.len());
    assert!(
        postings.iter().all(|posting| {
            posting.get("source").and_then(Value::as_str) == Some("newgrad_json")
        })
    );
    for posting in &postings {
        assert_posting_id(posting);
    }
    let ivo = postings
        .iter()
        .find(|posting| {
            posting.get("company").and_then(Value::as_str) == Some("Ivo")
                && posting.get("title").and_then(Value::as_str)
                    == Some("Software Engineer, Backend")
        })
        .expect("the Ivo backend posting");
    assert_eq!(
        ivo.get("url").and_then(Value::as_str),
        Some("https://jobs.lever.co/ivo/3ce9dc16-90fd-4b99-b4b9-dfd48ec1a50d")
    );
    assert_eq!(
        ivo.get("location").and_then(Value::as_str),
        Some("San Francisco, CA")
    );
}

#[test]
fn simplify_json_drops_a_closed_record() {
    let body = r#"[{"company_name":"Closed Co","title":"Engineer","active":false,"is_visible":true,"id":"x","url":"https://example.com"}]"#;
    let cfg: serde_yaml::Value = serde_yaml::from_str("id: newgrad_json").unwrap();
    assert!(simplify::parse_json(body, &cfg).is_empty());
}

#[test]
fn simplify_html_reads_the_derived_readme() {
    let body = fixture("simplify_list/README.html");
    let postings = simplify::parse_html(&body);
    assert!(postings.len() > 100, "only {} postings", postings.len());
    assert!(
        postings.iter().all(|posting| {
            posting.get("source").and_then(Value::as_str) == Some("newgrad_list")
        })
    );
    for posting in &postings {
        assert_posting_id(posting);
    }
}

#[test]
fn jobbank_search_reads_one_article() {
    let body = fixture("jobbank_search/jobsearch.html");
    let listings = jobbank::parse_search(&body, "Vancouver, BC");
    assert_eq!(listings.len(), 25);
    let first = &listings[0];
    assert_eq!(first.jid, "50720915");
    assert_eq!(first.title, "software engineer");
    assert_eq!(first.company, "Wingmate Inc.");
    assert_eq!(first.location, "Toronto (ON)");
    assert_eq!(
        first.posted.map(|date| date.to_string()),
        Some("2026-11-09".into())
    );
    assert!(first.salary.as_deref().unwrap_or("").contains("100,000"));
    assert!(first.partner);
}

#[test]
fn jobbank_detail_reads_title_description_and_apply_link() {
    let body = fixture("jobbank_detail/jobposting-50720915.html");
    let detail = jobbank::parse_detail(&body);
    assert!(
        detail
            .title
            .as_deref()
            .is_some_and(|title| !title.is_empty())
    );
    assert!(
        detail
            .description
            .as_deref()
            .is_some_and(|text| text.len() > 100)
    );
    assert!(
        detail
            .apply_url
            .as_deref()
            .is_some_and(|url| url.starts_with("http"))
    );
}

#[test]
fn greenhouse_reads_every_job() {
    let body = fixture("ats_greenhouse/databricks.json");
    let postings = ats::parse_greenhouse(&body, "Databricks");
    assert_eq!(postings.len(), 893);
    let first = &postings[0];
    assert_eq!(
        first.get("location").and_then(Value::as_str),
        Some("Tokyo, Japan")
    );
    assert_eq!(
        first.get("posted").and_then(Value::as_str),
        Some("2026-09-16")
    );
    assert_eq!(
        first.get("raw_id").and_then(Value::as_str),
        Some("8805655002")
    );
    assert_eq!(first.get("ats").and_then(Value::as_str), Some("greenhouse"));
}

#[test]
fn ashby_reads_a_truncated_board() {
    let body = fixture("ats_ashby/openai.json");
    let postings = ats::parse_ashby(&body, "OpenAI");
    assert_eq!(postings.len(), 259);
    let first = &postings[0];
    assert_eq!(
        first.get("title").and_then(Value::as_str),
        Some("Technical Program Manager, Compute Infrastructure")
    );
    assert_eq!(
        first.get("posted").and_then(Value::as_str),
        Some("2026-11-05")
    );
    assert_eq!(
        first.get("url").and_then(Value::as_str),
        Some("https://jobs.ashbyhq.com/openai/8fb1615c-34bf-47c4-a1d1-b7b2f836bbd3")
    );
}

#[test]
fn lever_reads_a_truncated_board() {
    let body = fixture("ats_lever/palantir.json");
    let postings = ats::parse_lever(&body, "Palantir");
    assert_eq!(postings.len(), 220);
    let first = &postings[0];
    assert_eq!(
        first.get("title").and_then(Value::as_str),
        Some("Administrative Business Partner")
    );
    assert_eq!(
        first.get("location").and_then(Value::as_str),
        Some("Singapore, Singapore")
    );
    assert_eq!(
        first.get("raw_id").and_then(Value::as_str),
        Some("6ed76ce8-4156-4b60-b120-403538bd66cd")
    );
    assert!(first.get("posted").and_then(Value::as_str).is_some());
}

#[test]
fn workable_reads_a_board() {
    let body = fixture("json_workable/accora.json");
    let postings = jsonboards::parse_page("workable", &body, "accora");
    assert_eq!(postings.len(), 4);
    let first = &postings[0];
    assert_eq!(
        first.get("title").and_then(Value::as_str),
        Some("Delivery & Support Technician")
    );
    assert_eq!(
        first.get("location").and_then(Value::as_str),
        Some("Dublin, County Dublin, Ireland")
    );
    assert_eq!(
        first.get("posted").and_then(Value::as_str),
        Some("2026-10-14")
    );
    assert_eq!(
        first.get("raw_id").and_then(Value::as_str),
        Some("C12BC36947")
    );
}

#[test]
fn smartrecruiters_uses_the_structured_country() {
    let body = fixture("json_smartrecruiters/3SBusinessCorporationInc1.json");
    let postings = jsonboards::parse_page("smartrecruiters", &body, "3SBusinessCorporationInc1");
    assert_eq!(postings.len(), 48);
    let first = &postings[0];
    assert_eq!(
        first.get("title").and_then(Value::as_str),
        Some("Excellent Job Opportunities in IT Industry @ 3S Busines Corporation.")
    );
    assert_eq!(
        first.get("location").and_then(Value::as_str),
        Some("Houston, TX, United States")
    );
    assert_eq!(
        first.get("raw_id").and_then(Value::as_str),
        Some("91183566")
    );
}

#[test]
fn rippling_is_undated() {
    let body = fixture("json_rippling/aalyria-careers.json");
    let postings = jsonboards::parse_page("rippling", &body, "aalyria-careers");
    assert_eq!(postings.len(), 32);
    let first = &postings[0];
    assert_eq!(
        first.get("title").and_then(Value::as_str),
        Some("Communications Systems Test Engineer")
    );
    assert!(first.get("posted").is_none_or(Value::is_null));
    assert_eq!(
        first.get("date_missing").and_then(Value::as_bool),
        Some(true)
    );
}

#[test]
fn bamboohr_is_undated() {
    let body = fixture("json_bamboohr/careers-list.json");
    let postings = jsonboards::parse_page("bamboohr", &body, "adidev");
    assert_eq!(postings.len(), 25);
    let first = &postings[0];
    assert_eq!(
        first.get("title").and_then(Value::as_str),
        Some("Entry -Mid Android Developer")
    );
    assert_eq!(
        first.get("location").and_then(Value::as_str),
        Some("Jersey City, New Jersey")
    );
    assert_eq!(
        first.get("url").and_then(Value::as_str),
        Some("https://adidev.bamboohr.com/careers/24/")
    );
}

#[test]
fn empty_and_failed_vendor_bodies_are_empty_boards() {
    assert!(jsonboards::parse_page("recruitee", r#"{"error":"Not Found"}"#, "1x").is_empty());
    assert!(jsonboards::parse_page("breezy", "[]", "hmt-tank").is_empty());
    assert!(
        jsonboards::parse_page(
            "teamtailor",
            r#"{"version":"https://jsonfeed.org/version/1.1","items":[]}"#,
            "noda-1720000970"
        )
        .is_empty()
    );
}

#[test]
fn gallery_reads_the_card_containers_and_time_attribute() {
    let body = fixture("startups_gallery/jobs.html");
    let postings = gallery::parse(&body);
    assert_eq!(postings.len(), 20);
    let first = &postings[0];
    assert_eq!(
        first.get("company").and_then(Value::as_str),
        Some("Lorikeet")
    );
    assert_eq!(
        first.get("title").and_then(Value::as_str),
        Some("Demand Generation")
    );
    assert_eq!(
        first.get("location").and_then(Value::as_str),
        Some("US - Remote")
    );
    assert_eq!(
        first.get("posted").and_then(Value::as_str),
        Some("2026-11-13")
    );
    assert_eq!(first.get("ats").and_then(Value::as_str), Some("ashby"));
    assert_eq!(
        first.get("raw_id").and_then(Value::as_str),
        Some("c4d5d6da-87f0-44c1-89cf-76958d7905b0")
    );
}

#[test]
fn workatastartup_reads_the_inertia_payload() {
    let body = fixture("workatastartup/jobs.html");
    let payload = workatastartup::payload(&body).expect("the data-page payload");
    let jobs = payload
        .get("props")
        .and_then(|props| props.get("jobs"))
        .and_then(Value::as_array)
        .expect("jobs");
    assert_eq!(jobs.len(), 30);
    assert_eq!(jobs[0].get("id").and_then(Value::as_i64), Some(95530));
    assert_eq!(
        jobs[0].get("companyName").and_then(Value::as_str),
        Some("Instawork")
    );
}

#[test]
fn workday_reads_a_board_page_and_rejects_an_error_body() {
    let body = fixture("workday_board/ngc-jobs.json");
    let posts = workday::parse_posts(&body, 100_000);
    assert_eq!(posts.len(), 20);
    assert_eq!(
        posts[0].0.get("title").and_then(Value::as_str),
        Some("Principal / Senior Principal Systems Engineer - Line of Sight")
    );
    assert_eq!(posts[0].1, Some(0));
    let error = fixture("workday_board/jobs.json");
    assert!(workday::parse_posts(&error, 100_000).is_empty());
}
