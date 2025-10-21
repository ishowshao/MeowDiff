use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::util;

const LOCK_FILENAME: &str = "watch.lock";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockInfo {
    pub project_id: String,
    pub pid: i32,
    #[serde(with = "chrono::serde::ts_seconds")]
    pub started_at: DateTime<Utc>,
    pub tool_version: String,
}

pub struct WatchLock {
    path: PathBuf,
    active: bool,
}

impl WatchLock {
    pub fn acquire(meta_dir: &Path, project_id: &str) -> Result<Self> {
        util::ensure_dir(meta_dir)?;
        let path = meta_dir.join(LOCK_FILENAME);
        if path.exists() {
            if let Some(existing) = read_lock_file(&path)? {
                if is_process_alive(existing.pid) {
                    bail!(
                        "watch already running for project {} (pid {})",
                        existing.project_id,
                        existing.pid
                    );
                } else {
                    tracing::warn!(pid = existing.pid, "removing stale watch.lock");
                    fs::remove_file(&path).ok();
                }
            } else {
                fs::remove_file(&path).ok();
            }
        }
        let info = LockInfo {
            project_id: project_id.to_string(),
            pid: std::process::id() as i32,
            started_at: Utc::now(),
            tool_version: util::tool_version(),
        };
        write_lock_file(&path, &info)?;
        Ok(Self { path, active: true })
    }

    pub fn path(meta_dir: &Path) -> PathBuf {
        meta_dir.join(LOCK_FILENAME)
    }

    pub fn read(meta_dir: &Path) -> Result<Option<LockInfo>> {
        let path = Self::path(meta_dir);
        read_lock_file(&path)
    }

    pub fn release(mut self) {
        if self.active {
            if let Err(err) = fs::remove_file(&self.path) {
                tracing::warn!(error = %err, "failed to remove watch.lock");
            }
            self.active = false;
        }
    }
}

impl Drop for WatchLock {
    fn drop(&mut self) {
        if self.active {
            fs::remove_file(&self.path).ok();
            self.active = false;
        }
    }
}

pub fn send_terminate(pid: i32) -> Result<()> {
    unsafe {
        if libc::kill(pid, libc::SIGTERM) != 0 {
            let err = std::io::Error::last_os_error();
            if let Some(code) = err.raw_os_error() {
                if code == libc::ESRCH {
                    return Ok(());
                }
            }
            bail!("failed to send SIGTERM to {pid}: {err}");
        }
    }
    Ok(())
}

pub fn is_process_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    unsafe { libc::kill(pid, 0) == 0 }
}

fn read_lock_file(path: &Path) -> Result<Option<LockInfo>> {
    if !path.exists() {
        return Ok(None);
    }
    let file = fs::File::open(path)
        .with_context(|| format!("failed to open lock file {}", path.display()))?;
    let info: LockInfo = serde_json::from_reader(file)
        .with_context(|| format!("failed to parse lock info {}", path.display()))?;
    Ok(Some(info))
}

fn write_lock_file(path: &Path, info: &LockInfo) -> Result<()> {
    if let Some(parent) = path.parent() {
        util::ensure_dir(parent)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut file = fs::File::create(&tmp)
            .with_context(|| format!("failed to create {}", tmp.display()))?;
        let json = serde_json::to_vec_pretty(info)?;
        file.write_all(&json)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)
        .with_context(|| format!("failed to rename {} to {}", tmp.display(), path.display()))?;
    Ok(())
}
