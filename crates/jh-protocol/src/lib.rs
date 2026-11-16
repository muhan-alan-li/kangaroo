//! The command and event contract shared by the menu-bar server and the desktop app.
//!
//! Both processes serialise these types as one JSON object per line over a Unix
//! socket: the client writes [`Command`], the server writes [`EngineEvent`].

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The socket both processes use unless `JH_SOCKET_PATH` overrides it.
pub fn default_socket_path() -> PathBuf {
    std::env::var_os("JH_SOCKET_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data/state/jh.sock"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScanMode {
    Quick,
    Full,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub values: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Command {
    Scan {
        mode: ScanMode,
    },
    CancelScan,
    MakeBrief {
        posting_id: String,
    },
    /// Ask for one posting's full record: its text, its salary fields, and its
    /// brief. The dashboard lists keep only what the table draws.
    PostingDetail {
        posting_id: String,
    },
    MakeBriefs {
        posting_ids: Vec<String>,
    },
    Apply {
        posting_id: String,
    },
    Unapply {
        posting_ids: Vec<String>,
    },
    SaveSettings {
        settings: Settings,
    },
    RefreshConfig,
    /// Re-send the current dashboard snapshot. The event stream keeps no
    /// retained value, so a client that connects after the server's first
    /// emission asks for one with this command.
    Snapshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorKind {
    Busy,
    NotImplemented,
    General,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScanState {
    #[default]
    Idle,
    Running {
        mode: ScanMode,
    },
    Error(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LastScan {
    pub mode: String,
    pub status: String,
    pub started_at: String,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostingRow {
    pub id: String,
    pub company: String,
    pub title: String,
    pub location: Option<String>,
    pub url: Option<String>,
    pub source: String,
    pub ats: Option<String>,
    pub tier: Option<String>,
    pub score: Option<i64>,
    pub age_days: Option<i64>,
    pub posted: Option<String>,
    pub expires_on: Option<String>,
    pub brief_status: Option<String>,
    pub has_text: bool,
    pub applied_at: Option<String>,
}

/// The brief attached to one posting. `payload` is the stage-C object, so the
/// drawer can show the verdict without a second model call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BriefDetail {
    pub status: String,
    pub engine: Option<String>,
    pub generated_at: Option<String>,
    pub error: Option<String>,
    #[serde(default)]
    pub payload: serde_json::Value,
}

/// One posting, in full, for the detail drawer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostingDetail {
    pub row: PostingRow,
    pub raw_id: Option<String>,
    pub salary_raw: Option<String>,
    pub keywords: Vec<String>,
    pub skills_have: Vec<String>,
    pub skills_gap: Vec<String>,
    pub emails: Vec<String>,
    pub level_ii: bool,
    pub requires_us_work_auth: bool,
    pub below_comp_floor: bool,
    pub first_seen_at: Option<String>,
    pub last_seen_at: Option<String>,
    /// The cached posting text, or `None` when the source gave no text.
    pub description: Option<String>,
    pub description_source: Option<String>,
    pub brief: Option<BriefDetail>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DashboardSnapshot {
    pub live_postings: i64,
    pub seen_count: i64,
    pub last_scan: Option<LastScan>,
    pub database_path: String,
    pub has_database: bool,
    pub config_files: usize,
    /// The engine's scan state, so a client that connects after startup reports
    /// `Idle` instead of leaving its panels on a "starting" placeholder.
    pub scan_state: ScanState,
    #[serde(default)]
    pub open: Vec<PostingRow>,
    #[serde(default)]
    pub shortlisted: Vec<PostingRow>,
    #[serde(default)]
    pub applied: Vec<PostingRow>,
    /// True when a DeepSeek credential exists, so the app can gate the brief
    /// buttons without asking for one on every click.
    #[serde(default)]
    pub briefs_enabled: bool,
    #[serde(default)]
    pub board_total: i64,
    #[serde(default)]
    pub board_ok: i64,
    #[serde(default)]
    pub board_failing: i64,
    #[serde(default)]
    pub board_broken: i64,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EngineEvent {
    Dashboard(DashboardSnapshot),
    PostingDetail(PostingDetail),
    ScanStatus(ScanState),
    BriefUpdated {
        posting_id: String,
        state: String,
    },
    Error {
        kind: ErrorKind,
        message: String,
        source: String,
    },
}

/// One protocol line. Kept as a single enum so a malformed or unknown line is
/// easy to reject without guessing which direction it was meant for.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WireMessage {
    Command { command: Command },
    Event { event: EngineEvent },
}
