use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileOp {
    Added,
    Modified,
    Deleted,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileStats {
    pub added: usize,
    pub removed: usize,
    pub chunks: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileRecord {
    pub path: String,
    pub op: FileOp,
    pub before_sha: Option<String>,
    pub after_sha: Option<String>,
    pub stats: FileStats,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecordStats {
    pub files: usize,
    pub lines_added: usize,
    pub lines_removed: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordMeta {
    pub record_id: String,
    pub project_id: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub files: Vec<FileRecord>,
    pub stats: RecordStats,
    pub prev_record_id: Option<String>,
    pub tool_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEntry {
    pub record_id: String,
    pub timestamp: DateTime<Utc>,
    pub files: usize,
    pub lines_added: usize,
    pub lines_removed: usize,
    pub duration_ms: i64,
    pub notes: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SnapshotInfo {
    pub record_id: String,
    pub sha: String,
}
