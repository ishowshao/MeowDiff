use std::io::{Read, Write};

use anyhow::Result;
use chrono::{DateTime, Utc};
use similar::{ChangeTag, TextDiff};

use crate::models::{FileOp, FileRecord, FileStats, RecordStats};
use crate::util;

#[derive(Debug, Clone)]
pub struct FileInput {
    pub path: String,
    pub before: Option<Vec<u8>>,
    pub after: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct FileArtifact {
    pub record: FileRecord,
    pub patch: String,
    pub before_blob: Option<Vec<u8>>,
    pub after_blob: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct RecordArtifact {
    pub files: Vec<FileArtifact>,
    pub stats: RecordStats,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
}

pub fn build_file_artifact(input: FileInput) -> Result<Option<FileArtifact>> {
    let before = input.before;
    let after = input.after;

    if before.is_some() && after.is_some() && before == after {
        return Ok(None);
    }

    let (before_sha, before_blob) = match before {
        Some(ref bytes) => (Some(util::hash_bytes(bytes)), Some(bytes.clone())),
        None => (None, None),
    };
    let (after_sha, after_blob) = match after {
        Some(ref bytes) => (Some(util::hash_bytes(bytes)), Some(bytes.clone())),
        None => (None, None),
    };

    if before_sha.is_some() && after_sha.is_some() && before_sha == after_sha {
        return Ok(None);
    }

    let op = match (before_sha.is_some(), after_sha.is_some()) {
        (false, true) => FileOp::Added,
        (true, false) => FileOp::Deleted,
        (true, true) => FileOp::Modified,
        (false, false) => return Ok(None),
    };

    let (patch, stats) = build_patch(&input.path, before_blob.as_ref(), after_blob.as_ref())?;

    let record = FileRecord {
        path: input.path,
        op,
        before_sha,
        after_sha,
        stats,
    };

    Ok(Some(FileArtifact {
        record,
        patch,
        before_blob,
        after_blob,
    }))
}

fn build_patch(
    path: &str,
    before: Option<&Vec<u8>>,
    after: Option<&Vec<u8>>,
) -> Result<(String, FileStats)> {
    match (before, after) {
        (Some(old_bytes), Some(new_bytes)) => {
            let old_text = match std::str::from_utf8(old_bytes) {
                Ok(txt) => txt.to_string(),
                Err(_) => return Ok(binary_patch(path)),
            };
            let new_text = match std::str::from_utf8(new_bytes) {
                Ok(txt) => txt.to_string(),
                Err(_) => return Ok(binary_patch(path)),
            };
            let diff = TextDiff::from_lines(old_text.as_str(), new_text.as_str());
            let (added, removed) = count_line_changes(&diff);
            let chunks = diff.ops().len();
            let patch = diff
                .unified_diff()
                .header(&format!("a/{path}"), &format!("b/{path}"))
                .to_string();
            Ok((
                patch,
                FileStats {
                    added,
                    removed,
                    chunks,
                },
            ))
        }
        (None, Some(new_bytes)) => {
            let new_text = match std::str::from_utf8(new_bytes) {
                Ok(txt) => txt.to_string(),
                Err(_) => return Ok(binary_patch(path)),
            };
            let diff = TextDiff::from_lines("", new_text.as_str());
            let (added, _) = count_line_changes(&diff);
            let patch = diff
                .unified_diff()
                .header("/dev/null", &format!("b/{path}"))
                .to_string();
            Ok((
                patch,
                FileStats {
                    added,
                    removed: 0,
                    chunks: diff.ops().len(),
                },
            ))
        }
        (Some(old_bytes), None) => {
            let old_text = match std::str::from_utf8(old_bytes) {
                Ok(txt) => txt.to_string(),
                Err(_) => return Ok(binary_patch(path)),
            };
            let diff = TextDiff::from_lines(old_text.as_str(), "");
            let (_, removed) = count_line_changes(&diff);
            let patch = diff
                .unified_diff()
                .header(&format!("a/{path}"), "/dev/null")
                .to_string();
            Ok((
                patch,
                FileStats {
                    added: 0,
                    removed,
                    chunks: diff.ops().len(),
                },
            ))
        }
        (None, None) => Ok((String::new(), FileStats::default())),
    }
}

pub fn aggregate_stats(files: &[FileRecord]) -> RecordStats {
    let mut stats = RecordStats::default();
    stats.files = files.len();
    for file in files {
        stats.lines_added += file.stats.added;
        stats.lines_removed += file.stats.removed;
    }
    stats
}

pub fn compress_patch(patch: &str) -> Result<Vec<u8>> {
    let mut encoder = zstd::Encoder::new(Vec::new(), 0)?;
    encoder.write_all(patch.as_bytes())?;
    let data = encoder.finish()?;
    Ok(data)
}

pub fn decompress_patch(bytes: &[u8]) -> Result<String> {
    let mut decoder = zstd::Decoder::new(bytes)?;
    let mut output = String::new();
    decoder.read_to_string(&mut output)?;
    Ok(output)
}

fn count_line_changes<'a>(diff: &TextDiff<'a, 'a, 'a, str>) -> (usize, usize) {
    let mut added = 0usize;
    let mut removed = 0usize;
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => added += change.value().lines().count(),
            ChangeTag::Delete => removed += change.value().lines().count(),
            ChangeTag::Equal => {}
        }
    }
    (added, removed)
}

fn binary_patch(path: &str) -> (String, FileStats) {
    (
        format!("Binary file change: {path}\n"),
        FileStats {
            added: 0,
            removed: 0,
            chunks: 1,
        },
    )
}
