use anyhow::Result;
use sqlx::{FromRow, SqlitePool, sqlite::SqlitePoolOptions};

#[derive(Debug, Clone, FromRow)]
pub struct PostingRow {
    pub id: String,
    pub source: String,
    pub raw_id: Option<String>,
    pub company: String,
    pub title: String,
    pub location: Option<String>,
    pub url: Option<String>,
    pub posted: Option<String>,
    pub age_days: Option<i64>,
    pub tier: Option<String>,
    pub score: Option<i64>,
}

pub async fn posting(pool: &SqlitePool, id: &str) -> Result<Option<PostingRow>> {
    Ok(sqlx::query_as::<_, PostingRow>("SELECT id, source, raw_id, company, title, location, url, posted, age_days, tier, score FROM postings WHERE id = ?")
        .bind(id).fetch_optional(pool).await?)
}

pub async fn connect(path: &str) -> Result<SqlitePool> {
    let url = format!("sqlite://{}?mode=rwc", path);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await?;
    for pragma in [
        "PRAGMA journal_mode=WAL",
        "PRAGMA synchronous=NORMAL",
        "PRAGMA foreign_keys=ON",
        "PRAGMA busy_timeout=5000",
    ] {
        sqlx::query(pragma).execute(&pool).await?;
    }
    Ok(pool)
}

pub async fn migrate(pool: &SqlitePool) -> Result<()> {
    sqlx::migrate!("../../migrations").run(pool).await?;
    Ok(())
}
