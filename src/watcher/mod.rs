mod lock;
mod microbatch;
pub use lock::{is_process_alive, send_terminate, LockInfo, WatchLock};
pub use microbatch::Batch;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use blake3::Hasher;
use chrono::{DateTime, Utc};
use notify::{recommended_watcher, Event, RecommendedWatcher, RecursiveMode, Watcher};
#[cfg(unix)]
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::mpsc;

use crate::ignore::IgnoreMatcher;
use crate::models::{FileRecord, RecordMeta};
use crate::pipeline::{
    aggregate_stats, build_file_artifact, compress_patch, FileArtifact, FileInput,
};
use crate::storage::StorageEngine;
use crate::util;

const DEFAULT_WINDOW_MS: u64 = 50;

pub struct WatchOptions {
    pub project_root: PathBuf,
    pub window: Duration,
}

impl Default for WatchOptions {
    fn default() -> Self {
        Self {
            project_root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            window: Duration::from_millis(DEFAULT_WINDOW_MS),
        }
    }
}

pub async fn watch(options: WatchOptions) -> Result<()> {
    let project_root = util::resolve_project_root(Some(options.project_root))?;
    let storage = Arc::new(StorageEngine::open(&project_root)?);
    let ignore = Arc::new(IgnoreMatcher::new(&project_root)?);

    let meta_dir = storage.paths().meta_dir.clone();
    let lock = WatchLock::acquire(&meta_dir, storage.project_id())?;

    let (tx, mut rx) = mpsc::channel::<Event>(1024);
    let mut watcher = create_watcher(tx)?;
    watcher
        .watch(&project_root, RecursiveMode::Recursive)
        .with_context(|| format!("failed to watch {}", project_root.display()))?;

    tracing::info!(
        project_id = storage.project_id(),
        root = %project_root.display(),
        "watcher started"
    );

    #[cfg(unix)]
    {
        let mut ctrl_c = Box::pin(tokio::signal::ctrl_c());
        let mut sigterm_stream =
            signal(SignalKind::terminate()).context("failed to listen for SIGTERM")?;
        loop {
            tokio::select! {
                _ = &mut ctrl_c => {
                    tracing::info!("SIGINT received, shutting down watcher");
                    break;
                }
                _ = sigterm_stream.recv() => {
                    tracing::info!("SIGTERM received, shutting down watcher");
                    break;
                }
                batch = microbatch::next_batch(&mut rx, options.window) => {
                    match batch {
                        Some(batch) => {
                            if let Err(err) = process_batch(batch, project_root.clone(), storage.clone(), ignore.clone()) {
                                tracing::error!(error = %err, "failed to process batch");
                            }
                        }
                        None => break,
                    }
                }
            }
        }
    }

    #[cfg(not(unix))]
    {
        let mut ctrl_c = Box::pin(tokio::signal::ctrl_c());
        loop {
            tokio::select! {
                _ = &mut ctrl_c => {
                    tracing::info!("SIGINT received, shutting down watcher");
                    break;
                }
                batch = microbatch::next_batch(&mut rx, options.window) => {
                    match batch {
                        Some(batch) => {
                            if let Err(err) = process_batch(batch, project_root.clone(), storage.clone(), ignore.clone()) {
                                tracing::error!(error = %err, "failed to process batch");
                            }
                        }
                        None => break,
                    }
                }
            }
        }
    }
    lock.release();
    Ok(())
}

fn create_watcher(tx: mpsc::Sender<Event>) -> Result<RecommendedWatcher> {
    let watcher = recommended_watcher(move |res| match res {
        Ok(event) => {
            if let Err(err) = tx.blocking_send(event) {
                tracing::warn!(%err, "dropping fs event");
            }
        }
        Err(err) => tracing::error!(error = %err, "watch error"),
    })?;
    Ok(watcher)
}

fn process_batch(
    batch: microbatch::Batch,
    project_root: PathBuf,
    storage: Arc<StorageEngine>,
    ignore: Arc<IgnoreMatcher>,
) -> Result<()> {
    let unique_paths = collect_paths(&batch.events, &project_root, &ignore);
    if unique_paths.is_empty() {
        return Ok(());
    }

    let artifacts = build_artifacts(&unique_paths, &project_root, &storage)?;
    if artifacts.is_empty() {
        return Ok(());
    }

    let file_records: Vec<FileRecord> = artifacts.iter().map(|a| a.record.clone()).collect();
    let stats = aggregate_stats(&file_records);
    let prev_record_id = storage.latest_record_id()?;
    let record_id = generate_record_id(storage.project_id(), batch.started_at, &file_records);

    let meta = RecordMeta {
        record_id: record_id.clone(),
        project_id: storage.project_id().to_string(),
        started_at: batch.started_at,
        ended_at: batch.ended_at,
        files: file_records,
        stats,
        prev_record_id,
        tool_version: util::tool_version(),
    };

    let mut patch = String::new();
    for artifact in &artifacts {
        patch.push_str(&artifact.patch);
        if !artifact.patch.ends_with('\n') {
            patch.push('\n');
        }
        patch.push('\n');
    }

    if !patch.trim().is_empty() {
        println!(
            "record {} (files: {}, +{}, -{})",
            record_id, meta.stats.files, meta.stats.lines_added, meta.stats.lines_removed
        );
        print!("{}", patch);
    }

    let compressed_patch = compress_patch(&patch)?;
    storage.commit_record(&meta, &compressed_patch, &artifacts)?;
    storage.register_touch()?;
    tracing::info!(record_id = %meta.record_id, files = meta.files.len(), "recorded batch");
    Ok(())
}

fn collect_paths(
    events: &[Event],
    project_root: &Path,
    ignore: &IgnoreMatcher,
) -> BTreeSet<String> {
    let mut paths = BTreeSet::new();
    for event in events {
        for path in &event.paths {
            if let Some(rel) = util::relative_path(project_root, path) {
                let abs = project_root.join(&rel);
                if !ignore.is_ignored(&abs, abs.is_dir()) {
                    paths.insert(rel);
                }
            }
        }
    }
    paths
}

fn build_artifacts(
    paths: &BTreeSet<String>,
    project_root: &Path,
    storage: &StorageEngine,
) -> Result<Vec<FileArtifact>> {
    let mut artifacts = Vec::new();
    for rel_path in paths.iter() {
        let absolute = project_root.join(rel_path);
        let after_blob = match fs::metadata(&absolute) {
            Ok(meta) => {
                if meta.is_dir() {
                    continue;
                }
                Some(fs::read(&absolute)?)
            }
            Err(_) => None,
        };
        let before_sha = storage.fetch_snapshot(rel_path)?;
        let before_blob = match before_sha {
            Some(ref sha) => Some(storage.read_blob(sha)?),
            None => None,
        };
        let input = FileInput {
            path: rel_path.clone(),
            before: before_blob,
            after: after_blob,
        };
        if let Some(artifact) = build_file_artifact(input)? {
            artifacts.push(artifact);
        }
    }
    Ok(artifacts)
}

fn generate_record_id(project_id: &str, started_at: DateTime<Utc>, files: &[FileRecord]) -> String {
    let mut hasher = Hasher::new();
    hasher.update(project_id.as_bytes());
    hasher.update(&started_at.timestamp_millis().to_be_bytes());
    for file in files {
        hasher.update(file.path.as_bytes());
        if let Some(ref sha) = file.after_sha {
            hasher.update(sha.as_bytes());
        }
    }
    let hash = hasher.finalize();
    let encoded = hex::encode(hash.as_bytes());
    encoded.chars().take(12).collect()
}
