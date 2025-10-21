use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use ignore::Match;

const DEFAULT_PATTERNS: &[&str] = &[
    ".git/",
    ".svn/",
    ".hg/",
    "node_modules/",
    "dist/",
    "build/",
    "coverage/",
    "__pycache__/",
    "venv/",
    ".venv/",
    ".idea/",
    ".vscode/",
    ".DS_Store",
    "target/",
];

#[derive(Clone)]
pub struct IgnoreMatcher {
    matcher: Gitignore,
    rules: Vec<String>,
    root: PathBuf,
}

impl IgnoreMatcher {
    pub fn new(project_root: &Path) -> Result<Self> {
        let mut builder = GitignoreBuilder::new(project_root);
        let mut rules = Vec::new();
        for pattern in DEFAULT_PATTERNS {
            builder
                .add_line(None, pattern)
                .with_context(|| format!("invalid default ignore pattern: {pattern}"))?;
            rules.push(pattern.to_string());
        }
        let custom = project_root.join(".meowdiffignore");
        if custom.exists() {
            if let Some(err) = builder.add(custom.as_path()) {
                return Err(anyhow::anyhow!(
                    "failed to parse {}: {}",
                    custom.display(),
                    err
                ));
            }
            rules.push(format!("(file) {}", custom.display()));
        }
        let matcher = builder
            .build()
            .map_err(|err| anyhow::anyhow!("failed to build ignore matcher: {err}"))?;
        Ok(Self {
            matcher,
            rules,
            root: project_root.to_path_buf(),
        })
    }

    pub fn is_ignored(&self, path: &Path, is_dir: bool) -> bool {
        match self.matcher.matched_path_or_any_parents(path, is_dir) {
            Match::None | Match::Whitelist(_) => false,
            Match::Ignore(_) => true,
        }
    }

    pub fn rules(&self) -> &[String] {
        &self.rules
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}
