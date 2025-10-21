use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::models::{FileOp, RecordMeta, RecordStats, TimelineEntry};
use crate::pipeline::FileArtifact;
use crate::util;

const META_VERSION: &str = "1";

pub struct StorageEngine {
    project_id: String,
    project_root: PathBuf,
    paths: StoragePaths,
    conn: Mutex<Connection>,
}

#[derive(Clone)]
pub struct StoragePaths {
    pub project_dir: PathBuf,
    pub records_dir: PathBuf,
    pub blobs_dir: PathBuf,
    pub meta_dir: PathBuf,
    pub timeline_db: PathBuf,
    pub registry_file: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectEntry {
    pub project_id: String,
    pub path: String,
    pub last_seen: i64,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct RegistryFile {
    projects: Vec<ProjectEntry>,
}

impl StorageEngine {
    pub fn open(project_root: &Path) -> Result<Self> {
        let project_root = project_root
            .canonicalize()
            .with_context(|| format!("failed to canonicalize {}", project_root.display()))?;
        let project_id = util::compute_project_id(&project_root)?;
        let meowdiff_root = util::meowdiff_root()?;
        let project_dir = meowdiff_root.join(&project_id);
        let records_dir = project_dir.join("records");
        let blobs_dir = project_dir.join("blobs");
        let meta_dir = project_dir.join("meta");
        let timeline_db = project_dir.join("timeline.db");
        let registry_file = meowdiff_root.join("registry.json");
        util::ensure_dir(&project_dir)?;
        util::ensure_dir(&records_dir)?;
        util::ensure_dir(&blobs_dir)?;
        util::ensure_dir(&meta_dir)?;

        let mut conn = Connection::open(&timeline_db)
            .with_context(|| format!("failed to open {}", timeline_db.display()))?;
        init_db(&mut conn)?;

        let engine = Self {
            project_id,
            project_root,
            paths: StoragePaths {
                project_dir,
                records_dir,
                blobs_dir,
                meta_dir,
                timeline_db,
                registry_file,
            },
            conn: Mutex::new(conn),
        };
        engine.persist_meta_version()?;
        engine.update_registry()?;
        Ok(engine)
    }

    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    pub fn paths(&self) -> &StoragePaths {
        &self.paths
    }

    pub fn latest_record_id(&self) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT record_id FROM records ORDER BY ts_end DESC LIMIT 1")?;
        let result = stmt
            .query_row([], |row| row.get::<_, String>(0))
            .optional()?;
        Ok(result)
    }

    pub fn register_touch(&self) -> Result<()> {
        self.update_registry()
    }

    pub fn commit_record(
        &self,
        meta: &RecordMeta,
        patch_bytes: &[u8],
        artifacts: &[FileArtifact],
    ) -> Result<()> {
        let record_dir = self.paths.records_dir.join(&meta.record_id);
        util::ensure_dir(&record_dir)?;
        let meta_path = record_dir.join("meta.json");
        let patch_path = record_dir.join("diff.patch.zst");

        // write meta json
        {
            let mut file = File::create(&meta_path)
                .with_context(|| format!("failed to create {}", meta_path.display()))?;
            serde_json::to_writer_pretty(&mut file, meta)?
        }

        {
            let mut file = File::create(&patch_path)
                .with_context(|| format!("failed to create {}", patch_path.display()))?;
            file.write_all(patch_bytes)?;
        }

        // ensure blobs
        for artifact in artifacts {
            if let Some(ref before_blob) = artifact.before_blob {
                if let Some(ref sha) = artifact.record.before_sha {
                    self.ensure_blob(sha, Some(before_blob))?;
                }
            }
            if let Some(ref after_blob) = artifact.after_blob {
                if let Some(ref sha) = artifact.record.after_sha {
                    self.ensure_blob(sha, Some(after_blob))?;
                }
            }
        }

        let files_json = serde_json::to_string(&meta.files)?;
        let stats_json = serde_json::to_string(&meta.stats)?;
        let diff_hash = util::hash_bytes(patch_bytes);

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO records (record_id, project_id, ts_start, ts_end, files_json, stats_json, prev_record_id, diff_hash, duration_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                meta.record_id,
                meta.project_id,
                meta.started_at.timestamp_millis(),
                meta.ended_at.timestamp_millis(),
                files_json,
                stats_json,
                meta.prev_record_id,
                diff_hash,
                (meta.ended_at - meta.started_at).num_milliseconds()
            ],
        )?;

        for file in &meta.files {
            match file.op {
                FileOp::Added | FileOp::Modified => {
                    if let Some(ref sha) = file.after_sha {
                        tx.execute(
                            "INSERT INTO latest_snapshots (path, sha, record_id, updated_at) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(path) DO UPDATE SET sha=excluded.sha, record_id=excluded.record_id, updated_at=excluded.updated_at",
                            params![
                                file.path,
                                sha,
                                meta.record_id,
                                meta.ended_at.timestamp_millis()
                            ],
                        )?;
                    }
                }
                FileOp::Deleted => {
                    tx.execute(
                        "DELETE FROM latest_snapshots WHERE path = ?1",
                        params![file.path],
                    )?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn read_record_meta(&self, record_id: &str) -> Result<RecordMeta> {
        let path = self.paths.records_dir.join(record_id).join("meta.json");
        let file =
            File::open(&path).with_context(|| format!("failed to open {}", path.display()))?;
        let meta: RecordMeta = serde_json::from_reader(file)
            .with_context(|| format!("failed to parse record metadata for {record_id}"))?;
        Ok(meta)
    }

    pub fn read_patch(&self, record_id: &str) -> Result<Vec<u8>> {
        let path = self
            .paths
            .records_dir
            .join(record_id)
            .join("diff.patch.zst");
        let mut file =
            File::open(&path).with_context(|| format!("failed to open diff for {record_id}"))?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;
        Ok(buf)
    }

    pub fn timeline(
        &self,
        limit: Option<usize>,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
    ) -> Result<Vec<TimelineEntry>> {
        let conn = self.conn.lock().unwrap();
        let mut sql =
            String::from("SELECT record_id, ts_end, stats_json, duration_ms FROM records");
        let mut clauses: Vec<String> = Vec::new();
        let mut args: Vec<i64> = Vec::new();
        if let Some(from_ts) = from {
            clauses.push("ts_end >= ?".into());
            args.push(from_ts.timestamp_millis());
        }
        if let Some(to_ts) = to {
            clauses.push("ts_end <= ?".into());
            args.push(to_ts.timestamp_millis());
        }
        if !clauses.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&clauses.join(" AND "));
        }
        sql.push_str(" ORDER BY ts_end DESC");
        if let Some(limit) = limit {
            sql.push_str(&format!(" LIMIT {limit}"));
        }
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query(rusqlite::params_from_iter(args.iter()))?;
        let mut entries = Vec::new();
        while let Some(row) = rows.next()? {
            let record_id: String = row.get(0)?;
            let ts_end: i64 = row.get(1)?;
            let stats_json: String = row.get(2)?;
            let duration_ms: i64 = row.get(3)?;
            let stats: RecordStats = serde_json::from_str(&stats_json)?;
            entries.push(TimelineEntry {
                record_id,
                timestamp: DateTime::<Utc>::from_timestamp_millis(ts_end)
                    .unwrap_or_else(|| Utc::now()),
                files: stats.files,
                lines_added: stats.lines_added,
                lines_removed: stats.lines_removed,
                duration_ms,
                notes: None,
            });
        }
        Ok(entries)
    }

    pub fn fetch_snapshot(&self, path: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT sha FROM latest_snapshots WHERE path = ?1")?;
        let result = stmt
            .query_row([path], |row| row.get::<_, String>(0))
            .optional()?;
        Ok(result)
    }

    pub fn read_blob(&self, sha: &str) -> Result<Vec<u8>> {
        let path = self.blob_path(sha);
        let mut reader =
            File::open(&path).with_context(|| format!("failed to open blob {}", path.display()))?;
        let mut decoder = zstd::Decoder::new(&mut reader)?;
        let mut buf = Vec::new();
        decoder.read_to_end(&mut buf)?;
        Ok(buf)
    }

    pub fn ensure_blob(&self, sha: &str, content: Option<&Vec<u8>>) -> Result<()> {
        let path = self.blob_path(sha);
        if path.exists() {
            return Ok(());
        }
        let data = content.context("blob content missing while attempting to persist new blob")?;
        if let Some(parent) = path.parent() {
            util::ensure_dir(parent)?;
        }
        let mut file = File::create(&path)
            .with_context(|| format!("failed to create blob {}", path.display()))?;
        let mut encoder = zstd::Encoder::new(&mut file, 0)?;
        encoder.write_all(data)?;
        encoder.finish()?;
        Ok(())
    }

    pub fn list_projects(&self) -> Result<Vec<ProjectEntry>> {
        let registry = self.read_registry()?;
        Ok(registry.projects)
    }

    fn read_registry(&self) -> Result<RegistryFile> {
        load_registry_file(&self.paths.registry_file)
    }

    pub fn update_registry(&self) -> Result<()> {
        let mut registry = self.read_registry()?;
        let entry = ProjectEntry {
            project_id: self.project_id.clone(),
            path: self.project_root.to_string_lossy().to_string(),
            last_seen: Utc::now().timestamp(),
        };
        registry
            .projects
            .retain(|p| p.project_id != self.project_id);
        registry.projects.push(entry);
        let path = &self.paths.registry_file;
        let mut file =
            File::create(path).with_context(|| format!("failed to write {}", path.display()))?;
        serde_json::to_writer_pretty(&mut file, &registry)?;
        Ok(())
    }

    fn blob_path(&self, sha: &str) -> PathBuf {
        let prefix = &sha[..2];
        self.paths.blobs_dir.join(prefix).join(format!("{sha}.zst"))
    }

    fn persist_meta_version(&self) -> Result<()> {
        let version_path = self.paths.meta_dir.join("version");
        if version_path.exists() {
            return Ok(());
        }
        let mut file = File::create(&version_path)?;
        file.write_all(META_VERSION.as_bytes())?;
        Ok(())
    }
}

fn init_db(conn: &mut Connection) -> Result<()> {
    conn.pragma_update(None, "journal_mode", &"WAL")?;
    conn.pragma_update(None, "synchronous", &"NORMAL")?;
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS records (
            record_id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            ts_start INTEGER NOT NULL,
            ts_end INTEGER NOT NULL,
            files_json TEXT NOT NULL,
            stats_json TEXT NOT NULL,
            prev_record_id TEXT,
            diff_hash TEXT NOT NULL,
            duration_ms INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS latest_snapshots (
            path TEXT PRIMARY KEY,
            sha TEXT NOT NULL,
            record_id TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        );
        "#,
    )?;
    Ok(())
}

fn load_registry_file(path: &Path) -> Result<RegistryFile> {
    if path.exists() {
        let file = File::open(path)?;
        let registry: RegistryFile =
            serde_json::from_reader(file).context("failed to parse registry.json")?;
        Ok(registry)
    } else {
        Ok(RegistryFile::default())
    }
}

pub fn read_registry_global() -> Result<Vec<ProjectEntry>> {
    let root = util::meowdiff_root()?;
    let path = root.join("registry.json");
    let registry = load_registry_file(&path)?;
    Ok(registry.projects)
}

pub fn find_project_entry(project_id: &str) -> Result<Option<ProjectEntry>> {
    let entries = read_registry_global()?;
    Ok(entries
        .into_iter()
        .find(|entry| entry.project_id == project_id))
}
