use regex::Regex;
use serde_yaml::Value;

fn get<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    v.get(key)
}
fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_sequence)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}
fn phrase_hit(text: &str, phrase: &str) -> bool {
    let p = phrase.trim();
    if p.is_empty() {
        return false;
    }
    let low = text.to_lowercase();
    let needle = p.to_lowercase();
    low.match_indices(&needle).any(|(i, _)| {
        let before = low[..i].chars().next_back();
        let after = low[i + needle.len()..].chars().next();
        !before.is_some_and(|c| c.is_ascii_alphanumeric())
            && !after.is_some_and(|c| c.is_ascii_alphanumeric())
    })
}
pub fn title_matches(title: &str, f: &Value) -> (bool, String) {
    for p in strings(get(f, "titles_exclude")) {
        if phrase_hit(title, &p) {
            return (false, format!("excluded:{p}"));
        }
    }
    for p in strings(get(f, "titles_include")) {
        if phrase_hit(title, &p) {
            return (true, format!("matched:{p}"));
        }
    }
    let role = strings(get(f, "titles_role_words"))
        .into_iter()
        .find(|p| phrase_hit(title, p));
    let tech = strings(get(f, "titles_tech_words"))
        .into_iter()
        .find(|p| phrase_hit(title, p));
    if let (Some(r), Some(t)) = (role, tech) {
        (true, format!("role+tech:{r}+{t}"))
    } else {
        (false, "no-title-match".into())
    }
}
pub fn is_level_ii(title: &str, f: &Value) -> bool {
    let Some(cfg) = get(f, "level_ii") else {
        return false;
    };
    if !cfg.get("enabled").and_then(Value::as_bool).unwrap_or(false) {
        return false;
    }
    if let Some(r) = cfg.get("level_i_range_pattern").and_then(Value::as_str)
        && Regex::new(r).ok().is_some_and(|re| re.is_match(title))
    {
        return false;
    }
    strings(cfg.get("patterns"))
        .iter()
        .any(|p| phrase_hit(title, p))
}
const STATES: &[&str] = &[
    "alabama",
    "alaska",
    "arizona",
    "arkansas",
    "california",
    "colorado",
    "connecticut",
    "delaware",
    "florida",
    "georgia",
    "hawaii",
    "idaho",
    "illinois",
    "indiana",
    "iowa",
    "kansas",
    "kentucky",
    "louisiana",
    "maine",
    "maryland",
    "massachusetts",
    "michigan",
    "minnesota",
    "mississippi",
    "missouri",
    "montana",
    "nebraska",
    "nevada",
    "new hampshire",
    "new jersey",
    "new mexico",
    "new york",
    "north carolina",
    "north dakota",
    "ohio",
    "oklahoma",
    "oregon",
    "pennsylvania",
    "rhode island",
    "south carolina",
    "south dakota",
    "tennessee",
    "texas",
    "utah",
    "vermont",
    "virginia",
    "washington",
    "west virginia",
    "wisconsin",
    "wyoming",
];
const US: &[&str] = &[
    "AL", "AK", "AZ", "AR", "CA", "CO", "CT", "DE", "FL", "GA", "HI", "ID", "IL", "IN", "IA", "KS",
    "KY", "LA", "ME", "MD", "MA", "MI", "MN", "MS", "MO", "MT", "NE", "NV", "NH", "NJ", "NM", "NY",
    "NC", "ND", "OH", "OK", "OR", "PA", "RI", "SC", "SD", "TN", "TX", "UT", "VT", "VA", "WA", "WV",
    "WI", "WY", "DC",
];
const CA: &[&str] = &[
    "BC", "AB", "SK", "MB", "ON", "QC", "NB", "NS", "PE", "NL", "YT", "NT", "NU",
];
fn boundary_token(text: &str, token: &str) -> bool {
    text.match_indices(token).any(|(i, _)| {
        let b = text[..i].chars().next_back();
        let a = text[i + token.len()..].chars().next();
        !b.is_some_and(|c| c.is_ascii_alphabetic()) && !a.is_some_and(|c| c.is_ascii_alphabetic())
    })
}
pub fn region_signals(location: &str) -> (bool, bool) {
    let low = location.to_lowercase();
    let ca = low.contains("canada")
        || low.contains("british columbia")
        || CA.iter().any(|t| boundary_token(location, t))
        || boundary_token(location, "CAN");
    let us = STATES.iter().any(|s| low.contains(s))
        || US.iter().any(|t| boundary_token(location, t))
        || ["US", "U.S", "U.S."]
            .iter()
            .any(|t| boundary_token(location, t))
        || Regex::new(r"(?i)(?:,|\-|\()\s*us\b")
            .unwrap()
            .is_match(location)
        || ["united states", "usa", "u.s."]
            .iter()
            .any(|s| low.contains(s));
    (ca, us)
}
type Tier = (
    String,
    usize,
    String,
    String,
    Option<String>,
    Vec<String>,
    Vec<String>,
);

pub fn tier_lookup(f: &Value) -> Vec<Tier> {
    get(f, "location_tiers")
        .and_then(Value::as_sequence)
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(i, t)| {
            Some((
                t.get("name")?.as_str()?.to_owned(),
                i,
                t.get("country")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_lowercase(),
                t.get("label")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                None,
                strings(t.get("cities")),
                strings(t.get("ambiguous_cities")),
            ))
        })
        .collect()
}
pub fn location_matches(location: Option<&str>, f: &Value) -> (bool, String, Option<String>) {
    let loc = location.unwrap_or("").trim();
    if loc.is_empty() {
        return (false, "unknown".into(), None);
    }
    let low = loc.to_lowercase();
    let (ca, us) = region_signals(loc);
    let wrong = us && !ca;
    for (name, _, country, _, _, cities, amb) in tier_lookup(f) {
        for city in cities {
            if low.contains(&city.to_lowercase())
                && ((country == "usa" && !ca) || (country != "usa" && !wrong))
            {
                return (true, "local".into(), Some(name.clone()));
            }
        }
        for city in amb {
            if low.contains(&city.to_lowercase()) && ca {
                return (true, "local".into(), Some(name.clone()));
            }
        }
    }
    let legacy = get(f, "locations");
    if !wrong {
        for c in strings(legacy.and_then(|v| v.get("metro"))) {
            if low.contains(&c.to_lowercase()) {
                return (true, "local".into(), None);
            }
        }
    }
    if ca {
        for c in strings(legacy.and_then(|v| v.get("metro_ambiguous"))) {
            if low.contains(&c.to_lowercase()) {
                return (true, "local".into(), None);
            }
        }
    }
    let r = get(f, "remote");
    let remote = r
        .and_then(|x| x.get("enabled"))
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if remote
        && Regex::new(r"\b(remote|anywhere|wfh)\b")
            .unwrap()
            .is_match(&low)
    {
        let countries: Vec<_> = strings(r.and_then(|x| x.get("accept_countries")))
            .into_iter()
            .map(|s| s.to_lowercase())
            .collect();
        if ca
            || countries.iter().any(|c| {
                low.split(|ch: char| !ch.is_ascii_alphabetic())
                    .any(|part| part == c)
            })
        {
            return (
                true,
                "remote_ca".into(),
                r.and_then(|x| x.get("tier_canada"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            );
        }
        if wrong {
            if r.and_then(|x| x.get("accept_us"))
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                return (
                    true,
                    "remote_us".into(),
                    r.and_then(|x| x.get("tier_us"))
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                );
            }
            return (false, "remote-wrong-country".into(), None);
        }
        if r.and_then(|x| x.get("accept_unspecified"))
            .and_then(Value::as_bool)
            .unwrap_or(true)
        {
            return (
                true,
                "remote_unspecified".into(),
                r.and_then(|x| x.get("tier_unspecified"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            );
        }
        return (false, "remote-unspecified-rejected".into(), None);
    }
    (false, "location-rejected".into(), None)
}
