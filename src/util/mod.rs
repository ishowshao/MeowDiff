use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use blake3::Hasher;
use chrono::{DateTime, Utc};
use directories::BaseDirs;
use nu_ansi_term::Color;

pub fn resolve_project_root(path: Option<PathBuf>) -> Result<PathBuf> {
    let path = match path {
        Some(p) => p,
        None => std::env::current_dir().context("failed to get current working directory")?,
    };
    let canonical = path
        .canonicalize()
        .with_context(|| format!("failed to canonicalize project path: {}", path.display()))?;
    Ok(canonical)
}

pub fn compute_project_id(project_root: &Path) -> Result<String> {
    let canonical = project_root
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", project_root.display()))?;
    let mut hasher = Hasher::new();
    hasher.update(canonical.to_string_lossy().as_bytes());
    let hash = hasher.finalize();
    let encoded = hex::encode(hash.as_bytes());
    Ok(encoded.chars().take(12).collect())
}

pub fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Hasher::new();
    hasher.update(bytes);
    let hash = hasher.finalize();
    hex::encode(hash.as_bytes())
}

pub fn meowdiff_root() -> Result<PathBuf> {
    let base = BaseDirs::new().context("failed to locate home directory")?;
    let dir = base.home_dir().join(".meowdiff");
    if !dir.exists() {
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create {}", dir.display()))?;
    }
    Ok(dir)
}

pub fn ensure_dir(path: &Path) -> Result<()> {
    if !path.exists() {
        std::fs::create_dir_all(path)
            .with_context(|| format!("failed to create directory {}", path.display()))?;
    }
    Ok(())
}

pub fn now_utc() -> DateTime<Utc> {
    Utc::now()
}

pub fn relative_path(project_root: &Path, path: &Path) -> Option<String> {
    path.strip_prefix(project_root)
        .ok()
        .map(|p| p.to_string_lossy().to_string())
}

pub fn tool_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

pub fn colorize_patch(patch: &str) -> String {
    let mut result = String::with_capacity(patch.len() + patch.len() / 10);
    let lines: Vec<&str> = patch.lines().collect();
    for (idx, &line) in lines.iter().enumerate() {
        let colored = if line.starts_with('+') && !line.starts_with("+++") {
            Color::Green.paint(line).to_string()
        } else if line.starts_with('-') && !line.starts_with("---") {
            Color::Red.paint(line).to_string()
        } else {
            line.to_string()
        };
        result.push_str(&colored);
        if idx != lines.len() - 1 || patch.ends_with('\n') {
            result.push('\n');
        }
    }
    result
}
