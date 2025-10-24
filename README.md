# MeowDiff

MeowDiff is a local-first change tracker that complements Git and editor undo stacks. It watches your working tree, captures micro-diffs with timestamps, and lets you attribute each edit to the originating process—human or automated—without ever leaving your machine.

## Why MeowDiff
- Track every edit between commits, even rapid-fire save cycles and AI-assisted rewrites.
- Attribute changes by process ID to understand which tool touched a file.
- Review a line-level timeline, restore snapshots, or extract archived revisions instantly.
- Persist history in your home directory (`~/.meowdiff/<project-id>/`) so repositories stay clean and portable.

## Quick Start
1. Ensure a recent Rust toolchain (`rustup default stable`) and `cargo` are installed.
2. Clone this repository and build the CLI: `cargo build --release`.
3. Start watching a project from its root:
   ```bash
   cargo run -- watch --path .
   ```
   Add `--daemon` to keep the watcher running in the background.
4. Inspect history as you work:
   ```bash
   cargo run -- timeline --limit 10
   cargo run -- diff <record-id>
   cargo run -- restore <record-id> --apply
   ```

## Core Workflow
- **Watch:** The watcher streams filesystem events into the pipeline, batching them according to the `--window-ms` micro-batch interval.
- **Store:** Records, blobs, and metadata are persisted via the bundled SQLite engine under `~/.meowdiff/<project-id>/`.
- **Review:** Use `timeline`, `show`, and `diff` subcommands for inspection; `extract` recreates artifacts outside the project tree.
- **Manage:** `projects`, `status`, and `stop` help list active sessions, check daemon health, and terminate watchers safely.

## Development Guide
- `cargo check` keeps compilation fast while iterating; `cargo test` runs unit and CLI smoke suites (`tests/cli_smoke.rs`).
- Module layout: `src/cli/` (command surface), `src/runtime/` (tracing setup), `src/watcher/` (fs observers), `src/pipeline/` (diff ingestion), `src/storage/` (SQLite + blobs), `src/models/` (serde types), `src/util/` (helpers), `src/ignore/` (ignore rules).
- Data directories created under `~/.meowdiff/` are local state; never commit them. Use `StorageEngine::register_touch` and friends for programmatic access.

## Contributing
Please read `AGENTS.md` for detailed contributor expectations, coding standards, and review checklists before opening a pull request.

## 中文简介
MeowDiff 是一款本地优先的改动追踪工具，弥补 Git 与 IDE 撤销栈的空白。它持续监控工作副本，捕捉细粒度改动并打上时间戳，可选地标记写入进程（人类或自动化脚本），同时所有数据都保存在本地。

### 为什么选择 MeowDiff
- 捕捉提交之间的每一次保存与 AI 辅助改写，保留完整演化轨迹。
- 通过进程 ID 归因每次写入，快速判断文件由谁修改。
- 提供行级时间线、`diff` 与 `restore` 命令，随时回看或恢复某次改动。
- 历史记录存放于 `~/.meowdiff/<project-id>/`，不污染仓库且便于携带。

### 快速上手
1. 安装最新 Rust 工具链（`rustup default stable`）与 `cargo`。
2. 克隆仓库并构建 CLI：`cargo build --release`。
3. 在项目根目录启动监控：
   ```bash
   cargo run -- watch --path .
   ```
   如需后台运行可追加 `--daemon`。
4. 在开发过程中查看历史：
   ```bash
   cargo run -- timeline --limit 10
   cargo run -- diff <record-id>
   cargo run -- restore <record-id> --apply
   ```

### 核心流程
- **Watch（监听）**：Watcher 依据 `--window-ms` 微批配置归并文件事件并推送到流水线。
- **Store（存储）**：记录、二进制快照和元数据借助内置 SQLite 写入 `~/.meowdiff/<project-id>/`。
- **Review（回顾）**：使用 `timeline`、`show`、`diff` 命令排查或回溯；`extract` 可以导出历史版本。
- **Manage（管理）**：通过 `projects`、`status`、`stop` 列出活跃会话、检查守护进程并安全终止。

### 开发指引
- `cargo check` 适合快速验证类型安全；`cargo test` 覆盖单测与 CLI 冒烟用例（`tests/cli_smoke.rs`）。
- 模块划分：`src/cli/`（命令行入口）、`src/runtime/`（tracing 初始化）、`src/watcher/`（文件监控）、`src/pipeline/`（补丁生成）、`src/storage/`（SQLite + blobs）、`src/models/`（序列化结构）、`src/util/`（工具函数）、`src/ignore/`（忽略规则）。
- `~/.meowdiff/` 下的目录视为本地状态，不要提交。代码层面可通过 `StorageEngine` 工具方法访问。

### 贡献说明
提交 PR 前请阅读 `AGENTS.md`，了解代码风格、测试要求与评审流程；PR 描述需包含复现步骤及风险说明。
