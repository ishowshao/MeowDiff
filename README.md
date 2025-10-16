# MeowDiff

**中文简介**
MeowDiff 是一款本地改动追踪工具，弥补 Git 与 IDE 撤销栈的空白。它持续监听文件变化、生成行级快照与时间线，并可标注由哪个进程（人手或 AI agent）改动。数据完全本地保存，随时比对与回滚。

* 超越 Git：捕捉提交之间的**细碎改动**与反复修改
* 编辑器之外：记录 **AI/脚本/外部工具**写入
* 行级时间线：统一 diff、一键回看与恢复
* 改动归因（可选）：标注**哪个进程/用户**修改
* 本地优先：数据存于项目内 `.meowdiff/`，不出网

**English**
MeowDiff is a local change tracker that fills the gaps left by Git and IDE undo stacks. It continuously watches files, creates line-level snapshots and a timeline, and can attribute edits to the responsible process (human or AI agent). All data stays local for quick diffing and rollback.

* Beyond Git: capture **micro-edits** between commits
* Outside the editor: record writes from **AI/CLI/scripts**
* Line-level timeline: unified diffs, instant review & restore
* Attribution (optional): tag the **process/user** behind each edit
* Local-first: stores data in `.meowdiff/`, no network required
