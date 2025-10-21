use std::path::PathBuf;
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
use crate::watcher::{self, WatchOptions};

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
    Timeline(TimelineArgs),
    Show(ShowArgs),
    Diff(DiffArgs),
    Restore(RestoreArgs),
    Status(StatusArgs),
    Projects(ProjectsArgs),
    Inspect(InspectArgs),
    Ignore(IgnoreArgs),
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

pub async fn run_cli() -> Result<()> {
    let cli = Cli::parse();
    runtime::init_tracing(cli.verbose)?;
    match cli.command {
        Commands::Watch(args) => handle_watch(args).await,
        Commands::Timeline(args) => handle_timeline(args),
        Commands::Show(args) => handle_show(args),
        Commands::Diff(args) => handle_diff(args),
        Commands::Restore(args) => handle_restore(args),
        Commands::Status(args) => handle_status(args),
        Commands::Projects(args) => handle_projects(args),
        Commands::Inspect(args) => handle_inspect(args),
        Commands::Ignore(args) => handle_ignore(args.command),
    }
}

async fn handle_watch(args: WatchArgs) -> Result<()> {
    let project_root = util::resolve_project_root(args.path)?;
    let options = WatchOptions {
        project_root,
        window: Duration::from_millis(args.window_ms),
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
    let storage = open_storage(args.path)?;
    let compressed = storage.read_patch(&args.record_id)?;
    let patch = decompress_patch(&compressed)?;
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

fn handle_status(args: StatusArgs) -> Result<()> {
    let storage = open_storage(args.path)?;
    let latest = storage.latest_record_id()?;
    println!("Project: {}", storage.project_id());
    println!("Root: {}", storage.project_root().display());
    match latest {
        Some(id) => {
            let meta = storage.read_record_meta(&id)?;
            println!(
                "Last record: {} at {} ({} files)",
                id, meta.ended_at, meta.stats.files
            );
        }
        None => println!("No records yet"),
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
