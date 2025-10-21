# Repository Guidelines

## Project Structure & Module Organization
- `src/main.rs` and `src/lib.rs` expose the async CLI entrypoint; keep shared logic in `lib.rs` modules so the binary stays thin.
- Feature-specific code is grouped by folder (`src/cli/`, `src/runtime/`, `src/watcher/`, `src/pipeline/`, `src/storage/`, `src/models/`, `src/util/`, `src/ignore/`); add new modules beside their peers and re-export through `lib.rs`.
- Persistent data lands under `~/.meowdiff/<project-id>/` (SQLite, blobs, metadata). Treat those directories as local state and exclude them from commits.
- Integration smoke tests live in `tests/cli_smoke.rs`; place longer-form docs or flow notes in `docs/`. Avoid editing `target/` artifacts directly.

## Build, Test, and Development Commands
- `cargo check` — fast type and dependency verification; run before submitting patches.
- `cargo build --release` — produces `target/release/meowdiff` for manual installs or profiling.
- `cargo run -- watch --path .` — starts the watcher against the current project; add `--daemon` to background the process.
- `cargo run -- timeline --json` — inspect recorded events for debugging serialization issues.
- `cargo test` (or `cargo test --test cli_smoke -- --nocapture`) — executes unit and integration suites with CLI logging enabled.

## Coding Style & Naming Conventions
- Use Rust 2021 defaults: 4-space indentation, `snake_case` functions, `PascalCase` types, `SCREAMING_SNAKE_CASE` constants.
- Run `cargo fmt --all` before every commit; format pragma tweaks belong in `rustfmt.toml` if introduced.
- Prefer `anyhow::Result` + `?` for error propagation in CLI/runtime layers and `tracing` macros for logging.
- Keep public CLI surfaces declarative: arguments defined in `src/cli/` with Clap derive annotations, helpers in `src/util/`.

## Testing Guidelines
- Co-locate unit tests with modules; reserve the `tests/` directory for end-to-end CLI scenarios using `assert_cmd`, `predicates`, and temporary workspaces.
- When touching persistence or diff output, assert against the files produced in `~/.meowdiff/<project-id>/` or leverage `StorageEngine` helpers.
- Name test functions `test_<behavior>` and favor descriptive record IDs to keep snapshots understandable.
- New features should ship with at least one regression test or smoke check covering the watcher, pipeline, or storage path they affect.

## Commit & Pull Request Guidelines
- Follow the existing history: concise, present-tense summaries (often in Chinese), e.g. `增强 watcher 守护能力`. Keep one logical change per commit.
- Reference issues with `#<id>` in commit bodies when applicable, and document migrations whenever storage schemas evolve.
- Pull requests must outline behavior, include CLI reproduction steps, and attach screenshots or diff excerpts when user-visible output changes.
- Tag reviewers responsible for watcher, storage, or pipeline areas if you modify those modules, and call out risk mitigation (tests run, manual checks).

## Agent-Specific Instructions
- Agents should stop background daemons with `cargo run -- stop` after automated edits to avoid locking conflicts for humans.
- Record significant automated refactors by exporting a timeline snippet (`cargo run -- timeline --limit 5 --json`) and attaching it to the PR discussion for traceability.
- Avoid bulk rewriting generated blobs under `~/.meowdiff/`; prefer invoking the CLI to produce canonical patches instead of manual file edits.
