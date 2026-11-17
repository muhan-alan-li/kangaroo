//! Keywords, skills and outreach addresses read out of a posting's own text.
//! Nothing here writes state or touches the network.

use fancy_regex::Regex;
use serde_yaml::Value as Yaml;
use std::{collections::HashSet, path::Path, sync::OnceLock};

/// Canonical skill -> the spellings that should match it. The lookaround forms
/// are why this uses `fancy-regex` rather than the `regex` crate.
const SKILL_VOCAB: &[(&str, &[&str])] = &[
    ("TypeScript", &[r"type ?script", r"\bts\b"]),
    (
        "JavaScript",
        &[r"java ?script", r"(?<![\w.])js(?![\w.])", r"ecmascript"],
    ),
    ("React", &[r"\breact(?:\.?js)?\b", r"\breact native\b"]),
    ("Next.js", &[r"next\.?js"]),
    ("Vue", &[r"\bvue(?:\.js)?\b"]),
    ("Angular", &[r"\bangular\b"]),
    ("Svelte", &[r"\bsvelte\b"]),
    ("Tailwind", &[r"tailwind"]),
    ("Node.js", &[r"node\.?js", r"\bnode\b"]),
    ("Python", &[r"\bpython\b"]),
    ("Django", &[r"\bdjango\b"]),
    ("Flask", &[r"\bflask\b"]),
    ("FastAPI", &[r"fast ?api"]),
    ("Java", &[r"\bjava\b(?! ?script)"]),
    ("Spring", &[r"spring ?boot", r"\bspring\b"]),
    ("Kotlin", &[r"\bkotlin\b"]),
    ("Scala", &[r"\bscala\b"]),
    ("C++", &[r"c\+\+", r"\bcpp\b"]),
    ("C#", &[r"c#", r"\.net\b", r"\bcsharp\b"]),
    ("C", &[r"(?<![a-z+#])c(?![a-z+#])\b(?=[ ,/])"]),
    ("Go", &[r"\bgolang\b", r"(?<![a-z])go(?![a-z])"]),
    ("Rust", &[r"\brust\b"]),
    ("Ruby", &[r"\bruby\b", r"rails"]),
    ("PHP", &[r"\bphp\b"]),
    ("Swift", &[r"\bswift\b"]),
    ("Kotlin/Android", &[r"\bandroid\b"]),
    ("iOS", &[r"\bios\b", r"\bswiftui\b", r"uikit"]),
    ("SQL", &[r"\bsql\b", r"pl/?sql"]),
    ("PostgreSQL", &[r"postgres(?:ql)?", r"\bpostgis\b"]),
    ("MySQL", &[r"\bmysql\b", r"\bmariadb\b"]),
    ("SQLite", &[r"\bsqlite\b"]),
    ("MongoDB", &[r"mongo"]),
    ("DynamoDB", &[r"dynamo ?db"]),
    ("Redis", &[r"\bredis\b"]),
    ("Elasticsearch", &[r"elastic ?search", r"\bopensearch\b"]),
    ("GraphQL", &[r"graph ?ql"]),
    ("REST APIs", &[r"rest(?:ful)? ?apis?", r"\brest\b"]),
    ("gRPC", &[r"\bgrpc\b"]),
    ("Kafka", &[r"\bkafka\b"]),
    ("RabbitMQ", &[r"rabbit ?mq"]),
    ("Spark", &[r"\bspark\b"]),
    ("Airflow", &[r"air ?flow"]),
    ("dbt", &[r"\bdbt\b"]),
    ("Snowflake", &[r"\bsnowflake\b"]),
    ("Databricks", &[r"databricks"]),
    (
        "ETL/pipelines",
        &[r"\betl\b", r"data pipelines?", r"\bingest(?:ion|ing)\b"],
    ),
    (
        "AWS",
        &[
            r"\baws\b",
            r"amazon web services",
            r"\blambda\b",
            r"\bs3\b",
            r"\bec2\b",
        ],
    ),
    ("GCP", &[r"\bgcp\b", r"google cloud", r"\bbigquery\b"]),
    ("Azure", &[r"\bazure\b"]),
    ("Docker", &[r"\bdocker\b", r"containers?"]),
    ("Kubernetes", &[r"kubernetes", r"\bk8s\b"]),
    (
        "Terraform",
        &[r"terraform", r"infrastructure as code", r"\biac\b"],
    ),
    (
        "CI/CD",
        &[r"ci/?cd", r"continuous (?:integration|delivery|deployment)"],
    ),
    ("GitHub Actions", &[r"github actions"]),
    ("Jenkins", &[r"\bjenkins\b"]),
    ("GitLab", &[r"\bgitlab\b"]),
    ("Git", &[r"(?<![a-z])git(?![a-z])", r"\bgithub\b"]),
    (
        "Linux",
        &[r"\blinux\b", r"\bunix\b", r"\bshell\b", r"\bbash\b"],
    ),
    ("Unit testing", &[r"unit tests?", r"unit testing"]),
    (
        "Integration testing",
        &[
            r"integration tests?",
            r"test automation",
            r"automated tests?",
        ],
    ),
    (
        "Cypress/Selenium",
        &[
            r"\bcypress\b",
            r"\bselenium\b",
            r"\bplaywright\b",
            r"\bjest\b",
            r"\bpytest\b",
        ],
    ),
    ("Debugging", &[r"debugging", r"troubleshoot"]),
    (
        "Observability",
        &[
            r"observability",
            r"monitoring",
            r"datadog",
            r"\bsplunk\b",
            r"prometheus",
            r"grafana",
            r"cloudwatch",
        ],
    ),
    ("Microservices", &[r"micro ?services?"]),
    ("Distributed systems", &[r"distributed systems?"]),
    (
        "System design",
        &[r"system design", r"software design", r"solution design"],
    ),
    ("Agile/Scrum", &[r"\bagile\b", r"\bscrum\b", r"\bkanban\b"]),
    ("Code review", &[r"code reviews?"]),
    ("Accessibility", &[r"\bwcag\b", r"accessib(?:ility|le)"]),
    (
        "Security",
        &[
            r"secure (?:coding|development|software)",
            r"application security",
            r"security best practices",
            r"\bowasp\b",
            r"\bwaf\b",
            r"vulnerabilit",
        ],
    ),
    (
        "Auth/identity",
        &[
            r"\boauth\b",
            r"\bsso\b",
            r"\bjwt\b",
            r"\brbac\b",
            r"authentication",
        ],
    ),
    (
        "Machine learning",
        &[
            r"machine learning",
            r"\bml\b",
            r"pytorch",
            r"tensorflow",
            r"scikit",
        ],
    ),
    (
        "LLM/agents",
        &[
            r"\bllms?\b",
            r"\brag\b",
            r"prompt engineering",
            r"ai agents?",
            r"agentic",
            r"vector (?:db|database|search|index)",
            r"embeddings",
        ],
    ),
    (
        "Data analysis",
        &[
            r"data analysis",
            r"analytics",
            r"pandas",
            r"\bsql quer(?:y|ies)\b",
        ],
    ),
    (
        "Product ownership",
        &[
            r"end.to.end ownership",
            r"own(?:ership)? (?:of )?(?:the )?feature",
            r"product.?(?:minded|driven|thinking)",
        ],
    ),
    (
        "Customer-facing",
        &[
            r"customer[- ]facing",
            r"work(?:ing)? (?:directly )?with customers",
            r"stakeholder",
        ],
    ),
    ("Frontend", &[r"front[- ]?end"]),
    ("Backend", &[r"back[- ]?end"]),
    ("Full-stack", &[r"full[- ]?stack"]),
    (
        "Mobile",
        &[
            r"mobile (?:app|development|platform)",
            r"\breact native\b",
            r"\bflutter\b",
        ],
    ),
    (
        "Web performance",
        &[
            r"web performance",
            r"performance optimi[sz]ation",
            r"lighthouse",
        ],
    ),
    ("API design", &[r"api design", r"design(?:ing)? apis?"]),
    (
        "Data modelling",
        &[r"data model", r"schema design", r"entity resolution"],
    ),
];

fn compiled() -> &'static Vec<(&'static str, Regex)> {
    static COMPILED: OnceLock<Vec<(&'static str, Regex)>> = OnceLock::new();
    COMPILED.get_or_init(|| {
        SKILL_VOCAB
            .iter()
            .filter_map(|(skill, alternatives)| {
                let pattern = format!("(?i){}", alternatives.join("|"));
                Regex::new(&pattern).ok().map(|regex| (*skill, regex))
            })
            .collect()
    })
}

/// The skills the candidate's profile claims, lowercased. A missing profile
/// yields an empty set: every keyword becomes a gap, which is noisy but never
/// wrong about what the resume says.
pub fn profile_skills(root: &Path) -> HashSet<String> {
    let path = root.join("data/candidate/candidate_profile.yaml");
    let Ok(text) = std::fs::read_to_string(path) else {
        return HashSet::new();
    };
    let Ok(profile) = serde_yaml::from_str::<Yaml>(&text) else {
        return HashSet::new();
    };
    let mut claimed = HashSet::new();
    if let Some(skills) = profile.get("skills").and_then(Yaml::as_mapping) {
        for value in skills.values() {
            if let Some(items) = value.as_sequence() {
                for item in items {
                    if let Some(item) = item.as_str() {
                        claimed.insert(item.to_lowercase());
                    }
                }
            }
        }
    }
    if let Some(titles) = profile
        .get("target")
        .and_then(|target| target.get("titles"))
        .and_then(Yaml::as_sequence)
    {
        for title in titles.iter().filter_map(Yaml::as_str) {
            claimed.insert(title.to_lowercase());
        }
    }
    claimed
}

pub fn extract_keywords(
    text: &str,
    claimed: &HashSet<String>,
    limit: usize,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    if text.trim().is_empty() {
        return (Vec::new(), Vec::new(), Vec::new());
    }
    let mut hits: Vec<String> = compiled()
        .iter()
        .filter(|(_, pattern)| pattern.is_match(text).unwrap_or(false))
        .map(|(skill, _)| (*skill).to_owned())
        .collect();
    if hits.iter().any(|hit| hit == "Full-stack") {
        hits.retain(|hit| hit != "Frontend" && hit != "Backend");
    }
    let have: Vec<String> = hits
        .iter()
        .filter(|skill| claimed_skill(skill, claimed))
        .cloned()
        .collect();
    let gap: Vec<String> = hits
        .iter()
        .filter(|skill| !have.contains(skill))
        .cloned()
        .collect();
    (
        hits.into_iter().take(limit).collect(),
        have.into_iter().take(limit).collect(),
        gap.into_iter().take(limit).collect(),
    )
}

fn claimed_skill(skill: &str, claimed: &HashSet<String>) -> bool {
    let low = skill.to_lowercase();
    claimed
        .iter()
        .any(|claimed| claimed == &low || claimed.contains(&low) || low.contains(claimed))
}

/// Published addresses in the text, best first. A named person's address ranks
/// above the employer's generic recruiting inbox.
pub fn extract_emails(text: &str) -> Vec<String> {
    static EMAIL: OnceLock<Regex> = OnceLock::new();
    static JUNK: OnceLock<Regex> = OnceLock::new();
    let email = EMAIL
        .get_or_init(|| Regex::new(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}").unwrap());
    let junk = JUNK.get_or_init(|| {
        Regex::new(r"(?i)(?:no-?reply|donotreply|do-not-reply|example\.(?:com|org)|sentry|wixpress|\.png|\.jpe?g|\.gif|\.svg|\.webp|\.css|\.js$|sentry\.io|@2x)").unwrap()
    });
    let inbox = [
        "careers",
        "jobs",
        "recruiting",
        "recruitment",
        "talent",
        "hr",
        "people",
        "apply",
        "contact",
        "hello",
        "info",
        "admin",
        "hiring",
    ];
    let mut found: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    let bytes = text.as_bytes();
    for hit in email.find_iter(text).flatten() {
        let addr = hit
            .as_str()
            .trim()
            .trim_matches(|c: char| ".,;:()<>\"'".contains(c))
            .to_owned();
        let low = addr.to_lowercase();
        if low.is_empty() || seen.contains(&low) || junk.is_match(&low).unwrap_or(false) {
            continue;
        }
        let end = hit.end();
        if end < bytes.len() && matches!(bytes[end], b'/' | b'{' | b'}') {
            continue;
        }
        seen.insert(low);
        found.push(addr);
    }
    found.sort_by_key(|addr| {
        let local = addr.split('@').next().unwrap_or("").to_lowercase();
        let generic = inbox.iter().any(|word| local.contains(word));
        (generic as u8, local.len(), addr.to_lowercase())
    });
    found.into_iter().take(4).collect()
}
