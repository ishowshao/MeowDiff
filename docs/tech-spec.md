# MeowDiff Technical Specification

## 1.目标概览
- 使用 Rust 实现的本地文件改动追踪器，支持 macOS 与 Linux。
- 提供实时监听、行级 diff 记录、快速查询与回滚能力。
- 数据存储于 `~/.meowdiff/<project-id>/`，满足 CLI 与未来 UI 的读取需求。

## 2.高层架构
```
┌──────────────┐      ┌──────────────┐      ┌──────────────┐
│ FS Watchers   │──▶──│ Event Pipeline│──▶──│ Storage Engine│
└─────▲────────┘      └──────▲───────┘      └──────▲───────┘
      │                       │                      │
      │                       │                      │
      │                 ┌─────┴─────┐          ┌─────┴─────┐
      │                 │ CLI Layer  │◀────────│ Query API  │
      │                 └───────────┘          └───────────┘
```

- **FS Watchers**：封装 `notify`（FSEvents/inotify），提供统一事件流。
- **Event Pipeline**：过滤忽略规则、按“事件驱动窗口”聚合、生成 diff 与元数据。
- **Storage Engine**：负责 blob 存储、记录元数据、时间线索引（SQLite）。
- **Query API**：对 CLI 提供 timeline/diff/restore 等查询能力。
- **CLI Layer**：基于 `clap` 的命令行接口，支持表格与 JSON 输出。

## 3.项目目录结构
- **根级元素**
  ```
  meowdiff/
    Cargo.toml
    Cargo.lock
    docs/
    src/
    tests/
    scripts/
    resources/ (可选)
  ```
  - `Cargo.toml`：定义 crate 依赖与构建配置。
  - `docs/`：存放需求、技术方案及其他设计文档。
  - `src/`：主代码库，按模块划分 watcher、pipeline、storage、CLI 等子目录。
  - `tests/`：高层集成测试与端到端验证。
  - `scripts/`：开发辅助脚本，例如启动/清理工具。
  - `resources/`：可选静态资源（默认忽略列表等），构建时通过 `include_str!` 嵌入。

- **其他约定**
  - 构建产物位于 `target/`（Rust 默认），纳入内置忽略列表。
  - 将来如需拆分 workspace，可在根目录新增 `Cargo.workspace.toml` 并重构目录，但首版保持单 crate。

## 4.核心依赖（初版）
- `notify`：跨平台文件监听。
- `ignore`：解析 `.meowdiffignore` 及内置默认规则。
- `tokio`：异步运行时，驱动事件管道与后台任务。
- `rusqlite`：时间线索引数据库。
- `serde`/`serde_json`：元数据序列化。
- `similar`：行级 diff 生成。
- `clap`：CLI 命令解析。
- `tracing` + `tracing-subscriber`：日志记录。
- `blake3`/`base62`：记录 ID、blob 命名。
- `zstd`（可选）：diff/blob 压缩。

## 5.CLI 命令契约
- **退出码约定**：0 表示成功；1 表示用户输入错误或命令前置条件不满足；2 表示运行时错误（监听异常、数据库故障等）。
- **`watch`**：
  - 参数：`[path]`，可选 `--daemon`、`--foreground`、`--no-ignore-cache`。
  - 输出：启动成功时打印 `project-id`、缓存目录、忽略规则摘要；若已在监听，提示现有进程信息并返回 1。
- **`stop`**：
  - 参数：`[path]` 或 `--project-id`。
  - 输出：停止成功提示 PID 与停止时间；若无运行实例返回 1。
- **`status`**：
  - 参数：`[path]`、`--json`。
  - 输出字段：`project_id`、`watching`（bool）、`pid`、`uptime`、`last_record_ts`、`storage_size`、`ignore_rules`（数量）。
- **`timeline`**：
  - 参数：`[path]`、`--limit`、`--from`、`--to`、`--json`。
  - 输出列：`record_id`、`timestamp`、`files`（数量）、`lines +/-`、`duration`、`notes`。
- **`show`**：
  - 参数：`<record-id>`、`--json`。
  - 输出：`meta.json` 展开的字段，默认以多行文本显示。
- **`diff`**：
  - 参数：`<record-id>`、`[file]`、`--stat`、`--json`。
  - 输出：统一 diff 文本或统计表；`--json` 时返回对象包含 `files`（数组，每项含 `path`,`added`,`removed`）。
- **`restore`**：
  - 参数：`<record-id>`、`--apply`、`--print`、`--yes`、`--force`。
  - 输出：若未 `--apply`，打印即将应用的文件列表及冲突提示；应用成功后返回 0，冲突时报错并返回 2。
- **`extract`**：
  - 参数：`<record-id>`、`--output <dir>`。
  - 输出：成功时列出导出的文件路径；目标目录存在冲突时返回 1。
- **`projects`**：
  - 输出列：`project_id`、`path`（最近一次使用的路径）、`last_seen`、`records`（计数）。
- **`inspect`**：
  - 参数：`<project-id>`、`--json`。
  - 输出：存储路径、累计记录数、首次/最近记录时间、存储大小。
- **`ignore list/test`**：
  - `list` 输出当前规则表；`test` 输出匹配结果（匹配/不匹配）并返回对应退出码（匹配=0，不匹配=1）。

## 6.守护与锁管理
- **锁粒度**：每个项目一把锁文件 `meta/watch.lock`，内容包含 `pid`、`started_at`、`cmdline`。`watch` 启动时若锁已存在且进程仍存活，则拒绝启动并提示现有实例。
- **守护模式**：
  - `--daemon` 下，父进程完成初始化后 `fork + setsid` 进入后台，子进程负责监听事件。
  - 捕获 `SIGTERM`、`SIGINT`、`SIGHUP`，优雅关闭 watcher，刷写 pending 记录并清理锁。
- **前台模式**：
  - 默认前台运行，支持 `Ctrl+C` 退出。`tokio::signal::ctrl_c` 触发时执行与守护模式一致的清理逻辑。
- **锁恢复**：
  - `status` 检查锁中 PID 是否存在；若进程已死，提示“stale lock”，可通过 `watch --force` 或 `stop --force` 清理。

## 7.SQLite 并发策略
- 使用单 `rusqlite::Connection`（线程安全模式）配合 `Arc<Mutex<_>>` 管理写入；查询命令创建独立只读连接。
- 启用 WAL 模式提升并发读取性能：`PRAGMA journal_mode=WAL;` `PRAGMA synchronous=NORMAL;`。
- 所有写入（记录插入、引用计数更新）封装在事务中；失败时 rollback 并重试最多一次。
- 崩溃恢复：启动时运行 `PRAGMA integrity_check;`；若失败，提示用户执行 `meowdiff repair`（后续实现）。

## 8.安全与权限
- 初次 `watch` 时验证目标路径为现有目录，拒绝监听 `~/.meowdiff/` 自身以防递归。
- macOS 需提示用户授予“完全磁盘访问”权限，否则某些目录不可读；检测常见错误码并输出引导信息。
- Linux 上若监听网络盘/虚拟文件系统，`notify` 可能退化为 polling；在日志中标记并提示可能的性能影响。
- 所有文件写入使用原子替换：先写入临时文件后 `rename`，避免中途崩溃损坏 blob/meta。


## 5.事件捕捉与微批归组
1. **监听启动**：`watch` 命令初始化 watcher，递归订阅项目路径，忽略默认目录及 `.meowdiffignore` 匹配项。
2. **原始事件**：每个文件写入触发 `notify::Event`（类型为 `Modify(Data)` 或 `Create`/`Remove`）。
3. **预过滤**：事件到达后立即以 `ignore::WalkBuilder` 匹配过滤；被忽略路径直接丢弃。
4. **微批策略**：
   - 使用 `tokio::time::sleep` + channel 模型。设置窗口基准 `WINDOW = 50ms`。
   - 当收到第一条事件时打开批次，记录 `start_ts`。
   - 批次内每次收到新事件，重置计时器为 `now + WINDOW`。
   - 若在 WINDOW 内无新事件到达，批次结束，生成一次记录。
   - 事件顺序保留，批次内允许多个文件/多次写同一文件。
5. **批次输出**：批次完成后，构造 `Batch` 对象（ID、开始/结束时间、事件明细）。传给 diff 生成阶段。

## 6.差异计算流程
对批次内每个文件执行以下步骤：
1. 读取当前文件内容（若文件被删除则记为 `None`）。
2. 查询上一条记录中该文件的 `after_sha`（缓存于内存 map，缺失时读取 SQLite）。若无历史视为新文件。
3. 获取旧内容：
   - 若 `before_sha` 存在于内存缓存，则直接使用。
   - 否则从 `blobs/<sha_prefix>/<sha>.zst` 解压获取。
4. 使用 `similar::TextDiff::from_lines(old, new)` 生成统一 diff，统计行增删、块数量。
5. 生成文件级记录项：
   ```json
   {
     "path": "src/main.rs",
     "op": "modify",
     "before_sha": "...",
     "after_sha": "...",
     "stats": {"added": 12, "removed": 4, "chunks": 3}
   }
   ```
6. 保存完整新内容至 blob（内容寻址，已存在则增加引用计数）。必要时同样保存 `before` blob，确保回滚时可取。
7. 将文件 diff 拼接至多文件 `diff.patch`，采用 gzip/zstd 压缩。

## 7.记录生成与存储
- **记录 ID**：`record_id = base62(blake3(batch_ts || paths || diff_hash))[0..12]`，可读且碰撞概率低。
- **目录结构**：
  ```
  ~/.meowdiff/<project-id>/
    timeline.db
    records/
      <record-id>/meta.json
      <record-id>/diff.patch.zst
    blobs/
      ab/abcdef1234...zst
    meta/
      version
      ignore_cache.json
  ```
- **meta.json schema**：
  ```json
  {
    "record_id": "rd_8k3fj2p4",
    "project_id": "6f3a2b17",
    "started_at": "2025-10-21T08:15:12.431Z",
    "ended_at": "2025-10-21T08:15:12.503Z",
    "files": [...],
    "stats": {"files": 2, "lines_added": 15, "lines_removed": 4},
    "prev_record_id": "rd_7x1c9bqa",
    "tool_version": "0.1.0"
  }
  ```
- **timeline.db schema**（SQLite）：
  ```sql
  CREATE TABLE records (
    record_id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    ts_start INTEGER NOT NULL,
    ts_end INTEGER NOT NULL,
    files_json TEXT NOT NULL,
    stats_json TEXT NOT NULL,
    prev_record_id TEXT,
    diff_hash TEXT NOT NULL
  );

  CREATE INDEX idx_records_ts ON records(project_id, ts_end DESC);
  CREATE INDEX idx_records_prev ON records(prev_record_id);
  ```
- **blobs/refs.db**（SQLite 或简单 JSON）记录 `<sha, ref_count>`，供日后垃圾回收使用。

## 8.查询与回滚流程
- **时间线查询**：`timeline` 通过 SQL 获取记录，`files_json` 展开后渲染表格；`--json` 直接输出查询结果。
- **记录详情**：`show` 读取 `meta.json`，若 `--json` 则原样输出；常规输出时格式化。
- **Diff 命令**：解压 `diff.patch.zst`，若指定 `--file`，依据 patch header 过滤展示。
- **回滚（restore --apply）**：
  1. 载入目标记录的 `files` 数组。
  2. 对每个文件计算当前内容 `current_sha`（读取并哈希）。
  3. 若 `current_sha != after_sha` 且 `--force` 未设定，则提示冲突并停止；或允许打印 diff。
  4. 从 blob 提取 `before` 内容（回到记录前状态）或 `after` 内容（回放生成状态），写回文件。
  5. 更新内存缓存及 `timeline.db` 中的“最新快照”表（可选 `latest_snapshots(path TEXT, sha TEXT)`）。
- **Dry run/print**：`restore --print` 输出统一 diff，供用户自行用 `patch`。

## 9.忽略规则处理
- 内置默认忽略列表在编译期写死，启动时加载。
- 项目根存在 `.meowdiffignore` 则解析追加，支持 `!pattern` 取消规则。
- `ignore_cache.json` 缓存合并结果的哈希；当文件或默认列表变动时自动刷新。
- `ignore list` 命令通过 `ignore` crate 的 matcher 展示当前生效规则；`ignore test` 复用 matcher 做单路径判断。

## 10.配置与状态文件
- 全局配置 `~/.meowdiff/config.toml`，字段示例：
  ```toml
  [runtime]
  window_ms = 50
  compression = "zstd"

  [default_ignore]
  extra = ["*.log"]
  ```
- 监听状态文件 `meta/watch.lock` 存储当前运行实例信息（PID、启动时间、CLI 版本）。`watch` 启动时检查锁防止重复进程。

## 11.日志与调试
- `tracing` 产生日志，写入 `meta/logs/current.log`（按日滚动）。
- 关键事件（批次生成、回滚失败）写入 `timeline.db` 的 `events` 表（可选），方便 CLI 查询近期错误。

## 12.测试策略
- **单元测试**：
  - 忽略匹配、微批窗口逻辑、diff 生成模块。
  - 存储层（blob 去重、meta 写入）使用内存临时目录。
- **集成测试**：
  - 使用 `tempfile` 创建临时项目，真实写文件触发 watcher（通过 `notify` 的 `RecommendedWatcher`）。
  - CLI 测试采用 `assert_cmd` 调用 `meowdiff` 二进制。
- **手动测试**：提供脚本快速启动 watcher、模拟 `npm install` 以验证忽略规则。

## 13.版本兼容与迁移
- `meta/version` 记录存储格式版本（例如 `1`）。升级时提供迁移脚本（遍历记录目录，重新生成 timeline.db）。
- CLI 在启动时检查版本不匹配，提示用户执行 `meowdiff migrate`（后续实现）。

## 14.未来扩展留口
- 增加 fanotify/EndpointSecurity 支持以实现进程归因。
- 提供 REST/IPC 服务供 GUI 消费。
- 历史快照压缩与自动清理策略（按 ref_count、空间阈值）。
- 跨路径导入：复制旧 `project-id` 目录并重建路径索引。
