//! Keyword and address extraction against the vocabulary the postings use.

use jh_core::keywords::{extract_emails, extract_keywords};
use std::collections::HashSet;

fn claimed(items: &[&str]) -> HashSet<String> {
    items.iter().map(|item| (*item).to_lowercase()).collect()
}

#[test]
fn keywords_split_have_from_gap() {
    let text = "We build REST APIs in Python and React, and stream events through Kafka.";
    let (all, have, gap) = extract_keywords(text, &claimed(&["Python", "React", "REST APIs"]), 24);
    assert!(all.iter().any(|skill| skill == "Python"));
    assert!(all.iter().any(|skill| skill == "React"));
    assert!(all.iter().any(|skill| skill == "Kafka"));
    assert!(have.contains(&"Python".to_string()));
    assert!(have.contains(&"React".to_string()));
    assert!(gap.contains(&"Kafka".to_string()));
    assert!(!gap.contains(&"Python".to_string()));
}

#[test]
fn a_full_stack_posting_drops_generic_family_words() {
    let (all, _, _) = extract_keywords(
        "A full-stack role touching frontend and backend services.",
        &claimed(&[]),
        24,
    );
    assert!(all.contains(&"Full-stack".to_string()));
    assert!(!all.contains(&"Frontend".to_string()));
    assert!(!all.contains(&"Backend".to_string()));
}

#[test]
fn no_text_means_no_keywords() {
    assert_eq!(
        extract_keywords("   ", &claimed(&[]), 24),
        (vec![], vec![], vec![])
    );
}

#[test]
fn emails_rank_a_person_before_a_recruiting_inbox() {
    let text =
        "Write to jobs@acme.com or jane.doe@acme.com. Ignore no-reply@acme.com and logo@2x.png.";
    let found = extract_emails(text);
    assert_eq!(found.first().map(String::as_str), Some("jane.doe@acme.com"));
    assert!(found.contains(&"jobs@acme.com".to_string()));
    assert!(!found.contains(&"no-reply@acme.com".to_string()));
    assert!(!found.iter().any(|address| address.contains("2x")));
}
