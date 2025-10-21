use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use clap::{ArgAction, Args, Parser, Subcommand};
use serde_json::{self, json};

use crate::ignore::IgnoreMatcher;
use crate::models::TimelineEntry;
use crate::pipeline::decompress_patch;
use crate::runtime;
use crate::storage::{find_project_entry, read_registry_global, StorageEngine};
use crate::util;
use crate::watcher::{self, is_process_alive, send_terminate, WatchLock, WatchOptions};

#[derive(Parser)]
#[command(author, version, about = "MeowDiff local change tracker")]
pub struct Cli {
    #[arg(short, long, action = ArgAction::Count, help = "Increase verbosity (-v, -vv)")]
    verbose: u8,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    Watch(WatchArgs),
    Stop(StopArgs),
    Timeline(TimelineArgs),
    Show(ShowArgs),
    Diff(DiffArgs),
    Restore(RestoreArgs),
    Status(StatusArgs),
    Projects(ProjectsArgs),
    Inspect(InspectArgs),
    Ignore(IgnoreArgs),
    Extract(ExtractArgs),
}

#[derive(Args)]
pub struct WatchArgs {
    #[arg(short, long, help = "Project path (defaults to CWD)")]
    pub path: Option<PathBuf>,
    #[arg(
        long,
        help = "Micro-batch window in milliseconds",
        default_value_t = 50
    )]
    pub window_ms: u64,
    #[arg(long, help = "Run watcher as background daemon")]
    pub daemon: bool,
    #[arg(long, hide = true)]
    pub foreground: bool,
}

#[derive(Args)]
pub struct StopArgs {
    #[arg(short, long, help = "Project path (defaults to CWD when omitted)")]
    pub path: Option<PathBuf>,
    #[arg(long, help = "Specify project-id instead of path")]
    pub project_id: Option<String>,
    #[arg(long, help = "Remove stale lock even if process is not running")]
    pub force: bool,
}

#[derive(Args)]
pub struct TimelineArgs {
    #[arg(short, long)]
    pub path: Option<PathBuf>,
    #[arg(long)]
    pub limit: Option<usize>,
    #[arg(long, value_name = "RFC3339")]
    pub from: Option<String>,
    #[arg(long, value_name = "RFC3339")]
    pub to: Option<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct ShowArgs {
    pub record_id: String,
    #[arg(short, long)]
    pub path: Option<PathBuf>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct DiffArgs {
    pub record_id: String,
    #[arg(short, long)]
    pub path: Option<PathBuf>,
    #[arg(long)]
    pub json: bool,
    #[arg(long)]
    pub stat: bool,
    #[arg(long)]
    pub file: Option<String>,
}

#[derive(Args)]
pub struct RestoreArgs {
    pub record_id: String,
    #[arg(short, long)]
    pub path: Option<PathBuf>,
    #[arg(long, help = "Apply changes instead of dry-run")]
    pub apply: bool,
}

#[derive(Args)]
pub struct StatusArgs {
    #[arg(short, long)]
    pub path: Option<PathBuf>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct ProjectsArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct InspectArgs {
    #[arg(long)]
    pub path: Option<PathBuf>,
    #[arg(long)]
    pub project_id: Option<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Subcommand)]
pub enum IgnoreCommands {
    List(IgnoreListArgs),
    Test(IgnoreTestArgs),
}

#[derive(Args)]
pub struct IgnoreArgs {
    #[command(subcommand)]
    pub command: IgnoreCommands,
}

#[derive(Args)]
pub struct IgnoreListArgs {
    #[arg(short, long)]
    pub path: Option<PathBuf>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct IgnoreTestArgs {
    #[arg(short, long)]
    pub project: Option<PathBuf>,
    pub target: PathBuf,
}

#[derive(Args)]
pub struct ExtractArgs {
    pub record_id: String,
    #[arg(short, long)]
    pub path: Option<PathBuf>,
    #[arg(long, value_name = "DIR")]
    pub output: PathBuf,
    #[arg(long)]
    pub overwrite: bool,
}

pub async fn run_cli() -> Result<()> {
    let cli = Cli::parse();
    runtime::init_tracing(cli.verbose)?;
    match cli.command {
        Commands::Watch(args) => handle_watch(args).await,
        Commands::Stop(args) => handle_stop(args),
        Commands::Timeline(args) => handle_timeline(args),
        Commands::Show(args) => handle_show(args),
        Commands::Diff(args) => handle_diff(args),
        Commands::Restore(args) => handle_restore(args),
        Commands::Status(args) => handle_status(args),
        Commands::Projects(args) => handle_projects(args),
        Commands::Inspect(args) => handle_inspect(args),
        Commands::Ignore(args) => handle_ignore(args.command),
        Commands::Extract(args) => handle_extract(args),
    }
}

async fn handle_watch(args: WatchArgs) -> Result<()> {
    let WatchArgs {
        path,
        window_ms,
        daemon,
        foreground,
    } = args;

    let project_root = util::resolve_project_root(path.clone())?;
    if daemon && !foreground {
        let exe = std::env::current_exe().context("failed to resolve current executable")?;
        let mut cmd = Command::new(exe);
        cmd.arg("watch")
            .arg("--foreground")
            .arg("--window-ms")
            .arg(window_ms.to_string())
            .arg("--path")
            .arg(project_root.to_string_lossy().to_string());
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = cmd.spawn().context("failed to spawn watcher daemon")?;
        println!(
            "Watcher daemon started (pid {}) for {}",
            child.id(),
            project_root.display()
        );
        return Ok(());
    }

    let options = WatchOptions {
        project_root,
        window: Duration::from_millis(window_ms),
    };
    watcher::watch(options).await
}

fn handle_timeline(args: TimelineArgs) -> Result<()> {
    let storage = open_storage(args.path)?;
    let from_ts = match args.from {
        Some(ref ts) => Some(parse_datetime(ts)?),
        None => None,
    };
    let to_ts = match args.to {
        Some(ref ts) => Some(parse_datetime(ts)?),
        None => None,
    };
    let entries = storage.timeline(args.limit, from_ts, to_ts)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&entries)?);
    } else {
        print_timeline(&entries);
    }
    Ok(())
}

fn handle_show(args: ShowArgs) -> Result<()> {
    let storage = open_storage(args.path)?;
    let meta = storage.read_record_meta(&args.record_id)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&meta)?);
    } else {
        println!("Record: {}", meta.record_id);
        println!("Project: {}", meta.project_id);
        println!("Started: {}", meta.started_at);
        println!("Ended:   {}", meta.ended_at);
        if let Some(prev) = meta.prev_record_id {
            println!("Previous: {}", prev);
        }
        println!(
            "Stats: files={}, +{}, -{}",
            meta.stats.files, meta.stats.lines_added, meta.stats.lines_removed
        );
        println!("Files:");
        for file in meta.files {
            println!("  - {} ({:?})", file.path, file.op);
        }
    }
    Ok(())
}

fn handle_diff(args: DiffArgs) -> Result<()> {
    let DiffArgs {
        record_id,
        path,
        json,
        stat,
        file,
    } = args;

    let storage = open_storage(path)?;
    let meta = storage.read_record_meta(&record_id)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&meta.files)?);
        return Ok(());
    }

    if stat {
        println!("Diff summary for record {}:", record_id);
        for entry in &meta.files {
            println!(
                "  - {:<40} {:>5} added {:>5} removed",
                entry.path, entry.stats.added, entry.stats.removed
            );
        }
        println!(
            "Totals: files={} +{} -{}",
            meta.stats.files, meta.stats.lines_added, meta.stats.lines_removed
        );
        return Ok(());
    }

    let compressed = storage.read_patch(&record_id)?;
    let mut patch = decompress_patch(&compressed)?;
    if let Some(filter) = file {
        patch = filter_patch_for_file(&patch, &filter);
        if patch.trim().is_empty() {
            println!("No diff found for {} in record {}", filter, record_id);
            return Ok(());
        }
    }

    println!("{}", patch);
    Ok(())
}

fn handle_restore(args: RestoreArgs) -> Result<()> {
    let RestoreArgs {
        record_id,
        path,
        apply,
    } = args;
    let storage = open_storage(path.clone())?;
    let project_root = util::resolve_project_root(path)?;
    let meta = storage.read_record_meta(&record_id)?;
    if !apply {
        println!("Would restore {} files:", meta.files.len());
        for file in &meta.files {
            println!("  - {}", file.path);
        }
        println!("Use --apply to write changes to disk.");
        return Ok(());
    }
    for file in &meta.files {
        let target = project_root.join(&file.path);
        match &file.after_sha {
            Some(sha) => {
                let data = storage.read_blob(sha)?;
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)
                        .with_context(|| format!("failed to create {}", parent.display()))?;
                }
                std::fs::write(&target, data)
                    .with_context(|| format!("failed to write {}", target.display()))?;
            }
            None => {
                if target.exists() {
                    std::fs::remove_file(&target)
                        .with_context(|| format!("failed to remove {}", target.display()))?;
                }
            }
        }
    }
    println!("Restored record {}", meta.record_id);
    Ok(())
}

fn handle_stop(args: StopArgs) -> Result<()> {
    let StopArgs {
        path,
        project_id,
        force,
    } = args;

    let (project_id, meta_dir) = if let Some(path) = path {
        let root = util::resolve_project_root(Some(path))?;
        let project_id = util::compute_project_id(&root)?;
        let meta_dir = util::meowdiff_root()?.join(&project_id).join("meta");
        (project_id, meta_dir)
    } else if let Some(requested_id) = project_id {
        let entry = find_project_entry(&requested_id)?
            .ok_or_else(|| anyhow!("project {requested_id} not found in registry"))?;
        let meta_dir = util::meowdiff_root()?.join(&entry.project_id).join("meta");
        (entry.project_id, meta_dir)
    } else {
        let root = util::resolve_project_root(None)?;
        let project_id = util::compute_project_id(&root)?;
        let meta_dir = util::meowdiff_root()?.join(&project_id).join("meta");
        (project_id, meta_dir)
    };

    let lock_info = match WatchLock::read(&meta_dir)? {
        Some(info) => info,
        None => {
            println!("No active watcher for project {project_id}");
            return Ok(());
        }
    };

    if is_process_alive(lock_info.pid) {
        send_terminate(lock_info.pid)?;
        println!("Sent SIGTERM to watcher pid {}", lock_info.pid);
    } else if !force {
        println!(
            "Watcher process {} not running; use --force to clear lock",
            lock_info.pid
        );
        return Ok(());
    } else {
        println!("Removing stale lock for project {project_id}");
    }

    let lock_path = WatchLock::path(&meta_dir);
    fs::remove_file(&lock_path).ok();
    println!("Stopped watcher for project {project_id}");
    Ok(())
}

fn handle_status(args: StatusArgs) -> Result<()> {
    let StatusArgs { path, json } = args;
    let storage = open_storage(path)?;
    let latest = storage.latest_record_id()?;
    let latest_meta = if let Some(ref id) = latest {
        Some(storage.read_record_meta(id)?)
    } else {
        None
    };
    let meta_dir = storage.paths().meta_dir.clone();
    let lock = WatchLock::read(&meta_dir)?;
    let watching = lock
        .as_ref()
        .map(|info| is_process_alive(info.pid))
        .unwrap_or(false);

    if json {
        let payload = json!({
            "project_id": storage.project_id(),
            "root": storage.project_root().to_string_lossy(),
            "watcher": {
                "active": watching,
                "lock": lock.clone(),
            },
            "latest_record": latest_meta.as_ref().map(|meta| json!({
                "record_id": meta.record_id,
                "ended_at": meta.ended_at,
                "files": meta.stats.files,
                "lines_added": meta.stats.lines_added,
                "lines_removed": meta.stats.lines_removed,
            })),
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        println!("Project: {}", storage.project_id());
        println!("Root: {}", storage.project_root().display());
        match &lock {
            Some(info) if watching => println!(
                "Watcher running (pid {}) since {}",
                info.pid, info.started_at
            ),
            Some(info) => println!("Watcher lock present but process {} not running", info.pid),
            None => println!("Watcher: inactive"),
        }
        if let Some(meta) = latest_meta {
            println!(
                "Last record: {} at {} (files: {}, +{}, -{})",
                meta.record_id,
                meta.ended_at,
                meta.stats.files,
                meta.stats.lines_added,
                meta.stats.lines_removed
            );
        } else {
            println!("No records yet");
        }
    }

    Ok(())
}

fn handle_projects(args: ProjectsArgs) -> Result<()> {
    let projects = read_registry_global()?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&projects)?);
    } else if projects.is_empty() {
        println!("No projects tracked yet");
    } else {
        println!("Known projects:");
        for entry in projects {
            println!("  - {} ({})", entry.project_id, entry.path);
        }
    }
    Ok(())
}

fn handle_inspect(args: InspectArgs) -> Result<()> {
    let storage = if let Some(path) = args.path {
        let root = util::resolve_project_root(Some(path))?;
        StorageEngine::open(&root)?
    } else if let Some(project_id) = args.project_id {
        let entry = find_project_entry(&project_id)?
            .ok_or_else(|| anyhow!("project {project_id} not found"))?;
        let entry_path = PathBuf::from(entry.path);
        StorageEngine::open(&entry_path)?
    } else {
        bail!("provide --path or --project-id")
    };
    let latest = storage.latest_record_id()?;
    let records = storage.timeline(None, None, None)?;
    if args.json {
        let payload = json!({
            "project_id": storage.project_id(),
            "root": storage.project_root(),
            "records": records.len(),
            "latest_record": latest,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        println!("Project: {}", storage.project_id());
        println!("Root: {}", storage.project_root().display());
        println!("Records: {}", records.len());
        if let Some(id) = latest {
            println!("Latest: {}", id);
        }
    }
    Ok(())
}

fn handle_ignore(cmd: IgnoreCommands) -> Result<()> {
    match cmd {
        IgnoreCommands::List(args) => {
            let root = util::resolve_project_root(args.path)?;
            let matcher = IgnoreMatcher::new(&root)?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(matcher.rules())?);
            } else {
                println!("Ignore rules for {}:", root.display());
                for rule in matcher.rules() {
                    println!("  - {}", rule);
                }
            }
            Ok(())
        }
        IgnoreCommands::Test(args) => {
            let root = util::resolve_project_root(args.project)?;
            let matcher = IgnoreMatcher::new(&root)?;
            let abs = if args.target.is_absolute() {
                args.target.clone()
            } else {
                root.join(&args.target)
            };
            let is_dir = abs.is_dir();
            let ignored = matcher.is_ignored(&abs, is_dir);
            let code = if ignored {
                println!("IGNORED");
                0
            } else {
                println!("TRACKED");
                1
            };
            std::process::exit(code);
        }
    }
}

fn handle_extract(args: ExtractArgs) -> Result<()> {
    let ExtractArgs {
        record_id,
        path,
        output,
        overwrite,
    } = args;

    let storage = open_storage(path)?;
    let meta = storage.read_record_meta(&record_id)?;
    util::ensure_dir(&output)?;

    for file in &meta.files {
        let Some(ref sha) = file.after_sha else {
            continue;
        };
        let data = storage.read_blob(sha)?;
        let dest = output.join(&file.path);
        if dest.exists() && !overwrite {
            bail!(
                "{} already exists; use --overwrite to replace",
                dest.display()
            );
        }
        if let Some(parent) = dest.parent() {
            util::ensure_dir(parent)?;
        }
        fs::write(&dest, data)
            .with_context(|| format!("failed to write extracted file {}", dest.display()))?;
    }

    println!("Extracted record {} to {}", record_id, output.display());
    Ok(())
}

fn open_storage(path: Option<PathBuf>) -> Result<StorageEngine> {
    let root = util::resolve_project_root(path)?;
    StorageEngine::open(&root)
}

fn parse_datetime(input: &str) -> Result<DateTime<Utc>> {
    let parsed = DateTime::parse_from_rfc3339(input)
        .with_context(|| format!("invalid RFC3339 timestamp: {input}"))?;
    Ok(parsed.with_timezone(&Utc))
}

fn print_timeline(entries: &[TimelineEntry]) {
    println!(
        "{:<14} {:<25} {:>5} {:>6} {:>6}",
        "Record", "Timestamp", "Files", "+", "-"
    );
    for entry in entries {
        println!(
            "{:<14} {:<25} {:>5} {:>6} {:>6}",
            entry.record_id.as_str(),
            entry.timestamp,
            entry.files,
            entry.lines_added,
            entry.lines_removed
        );
    }
}

fn filter_patch_for_file(patch: &str, file: &str) -> String {
    let needle_a = format!("a/{file}");
    let needle_b = format!("b/{file}");
    patch
        .split("\n\n")
        .filter(|section| section.contains(&needle_a) || section.contains(&needle_b))
        .collect::<Vec<_>>()
        .join("\n\n")
}
