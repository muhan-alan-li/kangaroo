use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use sqlx::{Row, SqlitePool};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Parser)]
#[command(name = "jh")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}
#[derive(Subcommand)]
enum Commands {
    Import {
        #[arg(long = "from")]
        from: PathBuf,
        #[arg(long)]
        db: PathBuf,
    },
}
#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Import { from, db } => {
            if let Some(parent) = db.parent().filter(|p| !p.as_os_str().is_empty()) {
                fs::create_dir_all(parent)?;
            }
            let pool = jh_db::connect(db.to_str().context("database path is not UTF-8")?).await?;
            jh_db::migrate(&pool).await?;
            import_all(&pool, &from).await?;
        }
    }
    Ok(())
}

fn read_json(path: &Path) -> Option<Value> {
    fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
}
fn lines(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}
fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_owned)
}
fn num(v: &Value, k: &str) -> Option<i64> {
    v.get(k).and_then(Value::as_i64)
}
async fn import_all(pool: &SqlitePool, dir: &Path) -> Result<()> {
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for table in [
        "postings",
        "scan_runs",
        "source_runs",
        "descriptions",
        "seen",
        "board_health",
        "cache",
        "harvested_boards",
        "probe_failures",
        "applied",
        "briefs",
        "brief_artifacts",
    ] {
        let _ = counts.insert(table.to_owned(), 0);
    }
    let posts = lines(&dir.join("portal_feed.jsonl"));
    if !posts.is_empty() {
        let mut tx = pool.begin().await?;
        for p in posts {
            let id = s(&p, "id").unwrap_or_default();
            if id.is_empty() {
                continue;
            }
            let r=sqlx::query("INSERT OR IGNORE INTO postings(id,source,ats,raw_id,company,title,location,url,salary_raw,posted,age_days,scanned_at,tier,score,level_ii,requires_us_work_auth,first_seen_at,last_seen_at,expires_on,retired_at,keywords_json,skills_have_json,skills_gap_json,emails_json,verified_emails_json,has_text,below_comp_floor,salary_annual_min,salary_annual_max,salary_estimated,posted_derived) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)")
 .bind(id).bind(s(&p,"source").unwrap_or_default()).bind(s(&p,"ats")).bind(s(&p,"raw_id")).bind(s(&p,"company").unwrap_or_default()).bind(s(&p,"title").unwrap_or_default()).bind(s(&p,"location")).bind(s(&p,"url")).bind(s(&p,"salary")).bind(s(&p,"posted")).bind(num(&p,"age_days")).bind(s(&p,"scanned_at")).bind(s(&p,"tier")).bind(num(&p,"score")).bind(p.get("level_ii").and_then(Value::as_bool).unwrap_or(false) as i64).bind(p.get("requires_us_work_auth").and_then(Value::as_bool).unwrap_or(false) as i64).bind(s(&p,"first_seen_at").unwrap_or_else(||s(&p,"scanned_at").unwrap_or_default())).bind(s(&p,"last_seen_at").unwrap_or_else(||s(&p,"scanned_at").unwrap_or_default())).bind(s(&p,"expires_on")).bind(s(&p,"retired_at")).bind(p.get("keywords").cloned().unwrap_or(json!([])).to_string()).bind(p.get("skills_have").cloned().unwrap_or(json!([])).to_string()).bind(p.get("skills_gap").cloned().unwrap_or(json!([])).to_string()).bind(p.get("emails").cloned().unwrap_or(json!([])).to_string()).bind(p.get("verified_emails").cloned().unwrap_or(json!([])).to_string()).bind(p.get("has_text").and_then(Value::as_bool).unwrap_or(false) as i64).bind(p.get("below_comp_floor").and_then(Value::as_bool).unwrap_or(false) as i64).bind(num(&p,"salary_annual_min")).bind(num(&p,"salary_annual_max")).bind(p.get("salary_estimated").and_then(Value::as_bool).unwrap_or(false) as i64).bind(p.get("posted_derived").and_then(Value::as_bool).unwrap_or(false) as i64).execute(&mut *tx).await?;
            counts
                .entry("postings".into())
                .and_modify(|n| *n += r.rows_affected() as usize)
                .or_insert(r.rows_affected() as usize);
        }
        tx.commit().await?;
    }
    import_seen(pool, dir, &mut counts).await?;
    import_applied(pool, dir, &mut counts).await?;
    import_misc(pool, dir, &mut counts).await?;
    for (t, n) in counts {
        println!("{t}: {n}");
    }
    Ok(())
}
async fn import_seen(
    pool: &SqlitePool,
    dir: &Path,
    c: &mut std::collections::BTreeMap<String, usize>,
) -> Result<()> {
    let rows = lines(&dir.join("seen.jsonl"));
    if rows.is_empty() {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    for v in rows {
        let id = s(&v, "id").unwrap_or_default();
        let r = sqlx::query(
            "INSERT OR IGNORE INTO seen(id,reported,first_seen_at,last_seen_at) VALUES(?,?,?,?)",
        )
        .bind(id)
        .bind(v.get("reported").and_then(Value::as_bool).unwrap_or(false) as i64)
        .bind(
            s(&v, "first_seen_at")
                .or_else(|| s(&v, "first_seen"))
                .unwrap_or_default(),
        )
        .bind(
            s(&v, "last_seen_at")
                .or_else(|| s(&v, "last_seen"))
                .unwrap_or_default(),
        )
        .execute(&mut *tx)
        .await?;
        *c.entry("seen".into()).or_default() += r.rows_affected() as usize;
    }
    tx.commit().await?;
    Ok(())
}
async fn import_applied(
    pool: &SqlitePool,
    dir: &Path,
    c: &mut std::collections::BTreeMap<String, usize>,
) -> Result<()> {
    let Some(v) = read_json(&dir.join("applied.json")) else {
        return Ok(());
    };
    let rows: Vec<(String, Value)> = if let Some(rows) = v
        .as_array()
        .or_else(|| v.get("applied").and_then(Value::as_array))
    {
        rows.iter()
            .map(|a| {
                (
                    s(a, "id")
                        .or_else(|| s(a, "posting_id"))
                        .unwrap_or_default(),
                    a.clone(),
                )
            })
            .collect()
    } else if let Some(rows) = v.as_object() {
        rows.iter().map(|(id, a)| (id.clone(), a.clone())).collect()
    } else {
        Vec::new()
    };
    if rows.is_empty() {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    for (id, a) in rows {
        let r = sqlx::query(
            "INSERT OR IGNORE INTO applied(posting_id,applied_at,snapshot_json) VALUES(?,?,?)",
        )
        .bind(id)
        .bind(s(&a, "applied_at").unwrap_or_default())
        .bind(a.to_string())
        .execute(&mut *tx)
        .await?;
        *c.entry("applied".into()).or_default() += r.rows_affected() as usize;
    }
    tx.commit().await?;
    Ok(())
}
async fn import_misc(
    pool: &SqlitePool,
    dir: &Path,
    c: &mut std::collections::BTreeMap<String, usize>,
) -> Result<()> {
    if let Some(d) = read_json(&dir.join("descriptions.json"))
        && let Some(m) = d.as_object()
    {
        let mut tx = pool.begin().await?;
        for (id, text) in m {
            if let Some(t) = text.as_str() {
                let r =
                    sqlx::query("INSERT OR IGNORE INTO descriptions(posting_id,text) VALUES(?,?)")
                        .bind(id)
                        .bind(t)
                        .execute(&mut *tx)
                        .await?;
                *c.entry("descriptions".into()).or_default() += r.rows_affected() as usize;
            }
        }
        tx.commit().await?
    }
    if let Some(data) = read_json(&dir.join("board_health.json")) {
        let boards = data.get("boards").and_then(Value::as_object);
        if let Some(boards) = boards {
            let mut tx = pool.begin().await?;
            for (key, b) in boards {
                let r=sqlx::query("INSERT OR IGNORE INTO board_health(key,vendor,slug,tenant,wd,site,status,consecutive_failures,last_ok_at,last_checked_at,next_check_at,last_error,error_kind,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)")
                    .bind(key).bind(s(b,"vendor")).bind(s(b,"slug")).bind(s(b,"tenant")).bind(s(b,"wd")).bind(s(b,"site")).bind(s(b,"status")).bind(num(b,"consecutive_failures").unwrap_or(0)).bind(s(b,"last_ok_at")).bind(s(b,"last_checked_at")).bind(s(b,"next_check_at")).bind(s(b,"last_error")).bind(s(b,"error_kind")).bind(s(b,"updated_at").or_else(||s(&data,"updated_at")).unwrap_or_default()).execute(&mut *tx).await?;
                *c.entry("board_health".into()).or_default() += r.rows_affected() as usize;
            }
            tx.commit().await?;
        }
    }
    if let Some(data) = read_json(&dir.join("workday_sweep.json")) {
        let updated = s(&data, "swept_at")
            .or_else(|| s(&data, "updated_at"))
            .unwrap_or_default();
        let r = sqlx::query(
            "INSERT OR IGNORE INTO cache(key,value_json,updated_at) VALUES('workday_sweep',?,?)",
        )
        .bind(data.to_string())
        .bind(updated)
        .execute(pool)
        .await?;
        *c.entry("cache".into()).or_default() += r.rows_affected() as usize;
    }
    if let Some(data) = read_json(&dir.join("sg_boards.json"))
        && let Some(boards) = data.get("boards").and_then(Value::as_array)
    {
        let mut tx = pool.begin().await?;
        for b in boards {
            let key = format!(
                "{}|{}",
                s(b, "ats").unwrap_or_default(),
                s(b, "slug").unwrap_or_default()
            );
            let r=sqlx::query("INSERT OR IGNORE INTO harvested_boards(key,source,payload_json,first_seen_at,last_seen_at) VALUES(?,?,?,?,?)").bind(key).bind("startups_gallery").bind(b.to_string()).bind(s(b,"first_seen").or_else(||s(&data,"updated_at")).unwrap_or_default()).bind(s(b,"last_seen").or_else(||s(&data,"updated_at")).unwrap_or_default()).execute(&mut *tx).await?;
            *c.entry("harvested_boards".into()).or_default() += r.rows_affected() as usize;
        }
        tx.commit().await?
    }
    if let Some(data) = read_json(&dir.join("ats_probe_failures.json"))
        && let Some(rows) = data.get("failures").and_then(Value::as_object)
    {
        let mut tx = pool.begin().await?;
        for (key, v) in rows {
            let streak = v.get("failures").and_then(Value::as_i64).unwrap_or(0);
            let r=sqlx::query("INSERT OR IGNORE INTO probe_failures(key,streak,last_error,updated_at) VALUES(?,?,?,?)").bind(key).bind(streak).bind(s(v,"last_error")).bind(s(&data,"updated_at").unwrap_or_default()).execute(&mut *tx).await?;
            *c.entry("probe_failures".into()).or_default() += r.rows_affected() as usize;
        }
        tx.commit().await?
    }
    if let Some(expired) = read_json(&dir.join("portal_expired.json"))
        && let Some(rows) = expired.as_object()
    {
        let mut tx = pool.begin().await?;
        for (id, v) in rows {
            let expires_on = s(v, "expires_on");
            sqlx::query("UPDATE postings SET retired_at=COALESCE(retired_at,?), expires_on=COALESCE(expires_on,?) WHERE id=? AND (retired_at IS NULL OR (expires_on IS NULL AND ? IS NOT NULL))").bind(s(v,"retired_at").or_else(||expires_on.clone()).unwrap_or_default()).bind(expires_on.clone()).bind(id).bind(expires_on).execute(&mut *tx).await?;
        }
        tx.commit().await?
    }
    let brief_dir = dir.join("briefs");
    if let Ok(entries) = fs::read_dir(&brief_dir) {
        let mut tx = pool.begin().await?;
        let mut paths: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
        paths.sort_by_key(|path| {
            path.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                n.ends_with(".raw.json")
                    || n.ends_with(".agent.json")
                    || n.ends_with(".review.json")
            })
        });
        for path in paths {
            let Some(name) = path.file_name().and_then(|v| v.to_str()) else {
                continue;
            };
            let Some(raw) = read_json(&path) else {
                continue;
            };
            let (id, kind) = if let Some(id) = name.strip_suffix(".raw.json") {
                (id, Some("raw"))
            } else if let Some(id) = name.strip_suffix(".agent.json") {
                (id, Some("agent"))
            } else if let Some(id) = name.strip_suffix(".review.json") {
                (id, Some("review"))
            } else if let Some(id) = name.strip_suffix(".json") {
                (id, None)
            } else {
                continue;
            };
            if let Some(kind) = kind {
                let r=sqlx::query("INSERT OR IGNORE INTO brief_artifacts(posting_id,kind,payload_json,created_at) VALUES(?,?,?,?)").bind(id).bind(kind).bind(raw.to_string()).bind(s(&raw,"generated_at").unwrap_or_default()).execute(&mut *tx).await?;
                *c.entry("brief_artifacts".into()).or_default() += r.rows_affected() as usize;
            } else {
                let posting_id = s(&raw, "posting_id").unwrap_or_else(|| id.to_owned());
                let r=sqlx::query("INSERT OR IGNORE INTO briefs(posting_id,status,engine,generated_at,started_at,payload_json,error) VALUES(?,?,?,?,?,?,?)").bind(posting_id).bind(s(&raw,"status").unwrap_or_else(||"unknown".into())).bind(s(&raw,"engine")).bind(s(&raw,"generated_at")).bind(s(&raw,"started_at")).bind(raw.to_string()).bind(s(&raw,"error")).execute(&mut *tx).await?;
                *c.entry("briefs".into()).or_default() += r.rows_affected() as usize;
            }
        }
        tx.commit().await?;
    }
    if let Some(scan) = read_json(&dir.join("last_scan.json")) {
        let started = s(&scan, "ran_at").unwrap_or_default();
        if !started.is_empty() {
            if let Some(matched) = scan.get("matched").and_then(Value::as_array) {
                let mut tx = pool.begin().await?;
                for p in matched {
                    let id = s(p, "id").unwrap_or_default();
                    if id.is_empty() {
                        continue;
                    }
                    let r=sqlx::query("INSERT OR IGNORE INTO postings(id,source,raw_id,company,title,location,url,salary_raw,posted,age_days,scanned_at,tier,score,first_seen_at,last_seen_at,expires_on,retired_at,keywords_json,emails_json,verified_emails_json) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)")
                        .bind(id).bind(s(p,"source").unwrap_or_default()).bind(s(p,"raw_id")).bind(s(p,"company").unwrap_or_default()).bind(s(p,"title").unwrap_or_default()).bind(s(p,"location")).bind(s(p,"url")).bind(s(p,"salary")).bind(s(p,"posted")).bind(num(p,"age_days")).bind(started.clone()).bind(s(p,"tier")).bind(num(p,"score")).bind(s(p,"first_seen_at").or_else(||s(p,"first_seen")).unwrap_or_else(||started.clone())).bind(s(p,"last_seen_at").or_else(||s(p,"last_seen")).unwrap_or_else(||started.clone())).bind(s(p,"expires_on")).bind(started.clone()).bind(p.get("keywords").cloned().unwrap_or(json!([])).to_string()).bind(p.get("emails").cloned().unwrap_or(json!([])).to_string()).bind(p.get("verified_emails").cloned().unwrap_or(json!([])).to_string()).execute(&mut *tx).await?;
                    *c.entry("postings".into()).or_default() += r.rows_affected() as usize;
                }
                tx.commit().await?;
            }
            let exists: i64 = sqlx::query("SELECT count(*) AS n FROM scan_runs WHERE started_at=?")
                .bind(&started)
                .fetch_one(pool)
                .await?
                .get("n");
            if exists == 0 {
                let mut tx = pool.begin().await?;
                let r=sqlx::query("INSERT INTO scan_runs(mode,status,started_at,finished_at,matched,stats_json) VALUES(?,?,?,?,?,?)").bind("full").bind("ok").bind(started).bind(s(&scan,"finished_at")).bind(scan.get("matched").and_then(Value::as_array).map(|a|a.len() as i64)).bind(scan.to_string()).execute(&mut *tx).await?;
                let run_id = r.last_insert_rowid();
                *c.entry("scan_runs".into()).or_default() += r.rows_affected() as usize;
                let found = scan.get("sources").and_then(Value::as_object);
                let seconds = scan.get("source_seconds").and_then(Value::as_object);
                if let Some(found) = found {
                    for (source, n) in found {
                        let r=sqlx::query("INSERT OR IGNORE INTO source_runs(scan_run_id,source,found,kept,seconds) VALUES(?,?,?,?,?)").bind(run_id).bind(source).bind(n.as_i64().unwrap_or(0)).bind(0_i64).bind(seconds.and_then(|m|m.get(source)).and_then(Value::as_f64).unwrap_or(0.0)).execute(&mut *tx).await?;
                        *c.entry("source_runs".into()).or_default() += r.rows_affected() as usize;
                    }
                }
                tx.commit().await?;
            }
        }
    }
    Ok(())
}
