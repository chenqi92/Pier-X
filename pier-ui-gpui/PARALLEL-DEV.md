# Pier-X GPUI 工具面板 · 多线程并行开发指南

本文档说明如何把右侧各个工具面板（Docker / 数据库 / SFTP / 日志 / Markdown 等）
拆分成相互独立、**互不冲突**的并行开发任务，并给出每个任务可直接粘贴给 AI Agent
的 prompt。

- 仓库根目录：`E:\workspace-freq\Pier-X`
- 开发子目录：`pier-ui-gpui`（GPUI 原生 UI 实验）
- 基线分支：`spike/gpui-migration`
- **所有并行任务的基线提交：`1ad739b`**

> 主分支 `main` 不受影响，所有工作都在 `spike/gpui-migration` 及其派生分支上进行。

---

## 1. 当前进度（已完成、已编译通过）

| 方向 | 状态 |
|---|---|
| 视觉像素级还原（真实 lucide 图标 + IBM Plex 字体 + 无边框自定义标题栏） | ✅ |
| 真实数据（Git 状态 / 文件列表 / 保存的服务器连接 / 多标签终端） | ✅ |
| Monitor 面板（真实本机 CPU/内存/交换/运行时长/进程数/系统信息） | ✅ |
| 性能 A/B 对比（M4 阶段已做） | ✅ |
| 其余工具面板（Docker / DB / SFTP / Logs / …） | 🟡 已搭好骨架，待填充 |

---

## 2. 架构说明（已搭好的并行开发地基）

骨架已经提交，**可编译运行**。每个工具面板都是 `pier-ui-gpui/src/panels/` 下的一个
独立文件，已在 `panels/mod.rs` 注册、并接入 shell。**每个开发者/Agent 只改一个文件**
（`src/panels/<名字>.rs`），永远不碰共享文件，所以并行开发不会产生冲突。

```
pier-ui-gpui/src/
├── main.rs            # 注册了 panels、ui 模块（勿改）
├── shell.rs           # 外壳：右侧面板分发到 panels 注册表（勿改）
│                      #   参考实现：monitor_panel（实时刷新）、git_panel（一次性取数）
├── ui.rs              # 共享设计组件（所有面板都用它，勿改）
│                      #   icon / panel_header / section_label / meter
│                      #   info_row / status_dot / empty_state / level_color
├── data.rs            # 数据访问层（勿改）：
│                      #   current_dir / list_dir / git_status / monitor_snapshot
│                      #   connections_raw（取原始 SshConfig 列表）
│                      #   connect_blocking（连一个保存的服务器 → SshSession）
└── panels/
    ├── mod.rs         # PanelViews 注册表 + for_svc 分发（已写好，勿改）
    ├── docker.rs      # ← 任务 D：只改这个
    ├── db.rs          # ← 任务 C：只改这个（MySQL/PG/Redis/SQLite 共用）
    ├── sftp.rs        # ← 任务 E：只改这个
    ├── logs.rs        # ← 任务 B：只改这个
    ├── markdown.rs    # ← 任务 A：只改这个
    ├── firewall.rs    # ← 其余：同模板
    ├── search.rs      # ← 其余：同模板
    ├── webserver.rs   # ← 其余：同模板
    └── software.rs    # ← 其余：同模板
```

每个面板文件是一个 gpui `View`：
- `pub struct XxxPanel { theme: Theme, /* 你的状态 */ }`
- `pub fn new(cx: &mut Context<Self>) -> Self`
- `impl Render for XxxPanel`

注册表已经把它实例化、并在对应工具被选中时显示出来，所以**你不需要改 shell/mod.rs**。

---

## 3. 数据来源分类（决定难度）

| 面板 | pier-core 入口 | 数据来源 |
|---|---|---|
| Markdown | `services::markdown::{load_file, render_html}` | 🟢 本地文件 |
| Logs | `logging::log_file_path()` + 读文件 | 🟢 本地文件 |
| SQLite（属于 DB） | `services::sqlite::SqliteClient::open(path)` | 🟢 本地文件 |
| Docker | `services::docker::list_containers_blocking(session, all)` | 🔴 需要 `SshSession` |
| MySQL/PG/Redis（DB） | `services::{mysql,postgres,redis}::*Client::connect_blocking` | 🔴 远程/隧道 |
| SFTP | `ssh::sftp`（`SftpClient`） | 🔴 需要 `SshSession` |
| Search / Firewall | `services::{code_search,firewall}::*_blocking(session)` | 🔴 需要 `SshSession` |

🔴 远程面板统一通过 `data::connections_raw()`（列出保存的连接）+
`data::connect_blocking(&cfg)`（建立 `SshSession`）来取数，**无需自己实现 SSH 认证**。

---

## 4. Git worktree + 分支设置（在 `E:\workspace-freq\Pier-X` 执行一次）

```powershell
# 每个面板一个 worktree + 分支（建在 Pier-X 的同级目录）
git worktree add ..\pier-x-markdown -b panel/markdown spike/gpui-migration
git worktree add ..\pier-x-logs     -b panel/logs     spike/gpui-migration
git worktree add ..\pier-x-db       -b panel/db       spike/gpui-migration
git worktree add ..\pier-x-docker   -b panel/docker   spike/gpui-migration
git worktree add ..\pier-x-sftp     -b panel/sftp     spike/gpui-migration

# 每个 worktree 都需要被 .gitignore 排除的 vendored zed 才能编译。
# 用 junction（无需管理员权限；symlink 需要开发者模式）：
cmd /c mklink /J E:\workspace-freq\pier-x-markdown\pier-ui-gpui\.vendor E:\workspace-freq\Pier-X\pier-ui-gpui\.vendor
cmd /c mklink /J E:\workspace-freq\pier-x-logs\pier-ui-gpui\.vendor     E:\workspace-freq\Pier-X\pier-ui-gpui\.vendor
cmd /c mklink /J E:\workspace-freq\pier-x-db\pier-ui-gpui\.vendor       E:\workspace-freq\Pier-X\pier-ui-gpui\.vendor
cmd /c mklink /J E:\workspace-freq\pier-x-docker\pier-ui-gpui\.vendor   E:\workspace-freq\Pier-X\pier-ui-gpui\.vendor
cmd /c mklink /J E:\workspace-freq\pier-x-sftp\pier-ui-gpui\.vendor     E:\workspace-freq\Pier-X\pier-ui-gpui\.vendor
```

> ⚠️ 每个 worktree 第一次 `cargo build` 会重新编译 gpui（约 10 分钟，独立 `target/`）。
> 这是并行隔离编译的固有成本；如果不想等，就改成一个一个串行编译。

---

## 5. 通用规则（粘给每个 Agent 时，放在最前面）

```
你在 Pier-X 仓库的一个 git worktree 里工作，分支 panel/<NAME>，
目录 <WORKTREE>\pier-ui-gpui。这是 Pier-X 的 GPUI（Rust）原生 UI 重写版，
基线提交 1ad739b 已经搭好面板系统。

硬性规则：
- 只允许修改 src/panels/<NAME>.rs 这一个文件。不要碰任何其他文件
  （panels/mod.rs、shell.rs、ui.rs、data.rs、Cargo.toml 都已接好）。
  如果你觉得需要改别的文件，说明思路错了——先停下来问。
- 你的面板已经注册好，对应工具被选中时会自动显示。它是一个 gpui View：
  pub struct <Name>Panel、pub fn new(cx: &mut Context<Self>) -> Self、
  impl Render。把它们填充完整即可。
- 只用 `cargo build`（在 <WORKTREE>\pier-ui-gpui 下）验证能编译通过。
  禁止启动程序、禁止截图、禁止运行 .exe。运行 GUI 的验证会在最后统一进行。
- 颜色/字体/尺寸只能用设计令牌：通过 self.theme（crate::theme::Theme）和
  crate::ui 里的共享组件（icon、panel_header、section_label、meter、
  info_row、status_dot、empty_state、level_color）。禁止硬编码 hex/rgb 颜色、
  字体名、或已有令牌的像素值。背景：t.bg/surface/panel/panel_2；
  文字：t.ink/ink_2/muted/dim；边框：t.line/line_2；强调色：t.accent；
  状态色：t.pos/neg/warn/info；间距 t.sp1..sp6；等宽字体 t.mono。
- 先研究参考实现：src/shell.rs::monitor_panel（真实数据 + 1.5s 刷新循环）、
  src/shell.rs::git_panel（一次性真实数据）、其他 src/panels/*.rs（View 骨架）、
  以及 src/ui.rs（组件库）。
- 绝不能在 render 路径里阻塞。render 只负责绘制。阻塞/IO 操作（SSH/DB 连接、
  查询、读文件）放到后台任务里，结果存进 View 状态，再 cx.notify()。模板：
      cx.spawn(async move |this, cx| {
          let result = cx.background_executor()
              .spawn(async move { /* 这里放阻塞调用 */ }).await;
          let _ = this.update(cx, |this, cx| { this.state = Some(result); cx.notify(); });
      }).detach();
- pier-core 保持与 UI 无关；直接当普通 Rust 依赖调用。不要往 Cargo.toml 加新依赖。
- 提交信息风格：feat(gpui): implement <name> panel，正文为客观事实条目，
  不要任何 AI/厂商署名，不要“优化/重构”这类主观词。
```

---

## 6. 各面板任务 prompt（接在通用规则后面）

### 任务 A — Markdown · 分支 `panel/markdown` · 目录 `..\pier-x-markdown` · 文件 `src/panels/markdown.rs`
```
目标：渲染一个 markdown 文件的内容。用 pier_core::services::markdown::load_file(path)
读取文件；render_html(source) 可用，但在 GPUI 里请把 markdown 解析成原生元素
（标题、段落、列表、代码块用 t.mono 放在 t.panel_2 上）。源文件：优先读
data::current_dir() 下的 CHANGELOG.md / README.md，没有则显示 ui::empty_state。
头部用 ui::panel_header(t, "file-text", "MARKDOWN", <文件名>)。正文要可滚动
（.id(..).overflow_y_scroll()）。解析可以简单，但要真实可用。
```

### 任务 B — Logs · 分支 `panel/logs` · 目录 `..\pier-x-logs` · 文件 `src/panels/logs.rs`
```
目标：实时查看 Pier-X 自己的日志文件。路径用 pier_core::logging::log_file_path() 获取。
读取最后约 500 行（放后台，别在 render 里读），按级别着色（ERROR→t.neg、
WARN→t.warn、INFO→t.info、DEBUG/TRACE→t.muted），等宽字体，可滚动，最新在底部。
仿照 monitor_panel 做一个约 1s 的受控刷新循环。
头部用 ui::panel_header(t, "scroll-text", "LOGS", <行数>)。没有日志文件则 ui::empty_state。
```

### 任务 C — 数据库 · 分支 `panel/db` · 目录 `..\pier-x-db` · 文件 `src/panels/db.rs`
```
目标：数据库浏览器。这一个文件同时服务 MySQL/Postgres/Redis/SQLite 四个工具。
先做不需要网络的本地路径：SQLite，用
pier_core::services::sqlite::SqliteClient::open(path) → .list_tables() →
.table_columns(table)。提供一个路径输入框（或默认 data::current_dir() 下的某个 .db），
渲染表列表 + 选中表的列信息。
远程引擎（MySQL/Postgres 用 *Client::connect_blocking(config)，Redis 用
RedisClient::connect_blocking）：加一个连接选择器，数据来自 data::connections_raw()；
在后台连接并列出数据库/表。
必须遵守只读默认（不做写入/DDL）。
头部用 ui::panel_header(t, "database", "DATABASE", ..)。
```

### 任务 D — Docker · 分支 `panel/docker` · 目录 `..\pier-x-docker` · 文件 `src/panels/docker.rs`
```
目标：列出选中服务器上的容器。先渲染一个连接选择器，数据来自
data::connections_raw()；选中后 data::connect_blocking(&cfg) 拿到 SshSession（放后台），
再调 pier_core::services::docker::list_containers_blocking(&session, true)。
每个容器渲染一行：状态点（用 ui::status_dot，Container::is_running() 为真则绿色）、
名称、镜像、端口——字段都看 pier-core/src/services/docker.rs 里的 Container 结构。
把 session 和列表缓存到 View 状态。
头部用 ui::panel_header(t, "container", "DOCKER", <数量>)。
连接/列举失败用 t.neg 的一行错误提示。本期只做只读列表，不实现 启动/停止。
```

### 任务 E — SFTP · 分支 `panel/sftp` · 目录 `..\pier-x-sftp` · 文件 `src/panels/sftp.rs`
```
目标：远程文件浏览器。连接选择器数据来自 data::connections_raw()；
data::connect_blocking(&cfg) 拿到 SshSession（放后台）。用 pier_core::ssh::sftp
（看 pier-core/src/ssh/sftp.rs 里的 SftpClient API）列出远程目录；
条目目录在前，渲染 ui::icon("folder"/"file") + 名称 + 大小，点击可进入目录，
并提供一行 ".." 返回上级。当前路径存进 View 状态。
头部用 ui::panel_header(t, "folder", "SFTP", <当前路径>)。本期只做只读浏览（不做 上传/下载）。
```

### 其余面板（Search / Firewall / Webserver / Software）
文件与注册表均已存在：`search.rs`、`firewall.rs`、`webserver.rs`、`software.rs`。
入口：`code_search::search_blocking(session, opts)`、`firewall::snapshot_blocking(session)`、
`web_server`/`nginx`/`apache`/`caddy`、`package_manager`/`package_mirror`。
全部是远程数据，复用任务 D 的 `connections_raw()` + `connect_blocking()` 模式。

---

## 7. 合并与验证

每个分支只改了自己那一个文件，所以合并无冲突：

```powershell
cd E:\workspace-freq\Pier-X
git checkout spike/gpui-migration
git merge panel/markdown
git merge panel/logs
git merge panel/db
git merge panel/docker
git merge panel/sftp
# 清理 worktree
git worktree remove ..\pier-x-markdown
git worktree remove ..\pier-x-logs
git worktree remove ..\pier-x-db
git worktree remove ..\pier-x-docker
git worktree remove ..\pier-x-sftp
```

合并完成后，在主 worktree 里统一编译并运行验证（一次性，集中进行）：

```powershell
cd E:\workspace-freq\Pier-X\pier-ui-gpui
cargo build
.\target\debug\pier-ui-gpui.exe
```

逐个点击右侧工具栏图标，检查每个面板的真实数据与交互。
