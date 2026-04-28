# Pier-X 产品规范

> 本文档是 Pier-X "这个软件是什么"的权威来源。
> 任何功能决策、panel 设计、交互流都必须对齐本文。
> 偏离本规范的实现（新增工具、变更默认行为、引入不兼容架构）视为需要先更新本文档再落代码。
>
> 视觉 token 见 [../.agents/skills/pier-design-system/SKILL.md](../.agents/skills/pier-design-system/SKILL.md)；代码规则见 [../CLAUDE.md](../CLAUDE.md)；前端 → 后端能力差距追踪见 [BACKEND-GAPS.md](BACKEND-GAPS.md)。

---

## 1. 产品定位

Pier-X 是一款桌面开发辅助工具，把**终端 / Git / SSH / 数据库 / 远程运维**放进一个 IDE 风格的工作台。对标参照对象是 JetBrains IDE 的工程感与一致性，而不是终端模拟器、SSH 客户端或数据库客户端的单一定位。

| 项 | 值 |
|---|---|
| 目标平台 | macOS + Windows（首发）；Linux 长期保留但不保证同等体验 |
| 目标用户 | 同时要用本地 shell、多台远程服务器、若干数据库的后端/运维工程师 |
| 技术栈 | Tauri 2 + React 19 + TypeScript（shell），Rust（`pier-core` 后端） |
| 非目标 | 浏览器版本、团队协作、云同步、AI 代码补全、插件市场（预留接口但首发不做） |

### 1.1 核心卖点（判断 feature 是否该做的准绳）

- **一站式**：从本地终端 → SSH → 远程 Git / DB / Docker / 监控，不切换工具。
- **IDE 质感**：快捷键、主题、密度、错误反馈都按 IDE 标准。
- **离线、本地**：默认不连任何外部服务；SSH 凭证在系统 keyring 里。
- **可见即可控**：所有危险操作（写 SQL、`git discard`、`docker rm`、SFTP delete）必须显式确认，不做自动幂等化兜底。

### 1.2 不做的事（任何 PR 都不应引入）

- 不引入浏览器/网页运行时、不引入 Node 服务端进程
- 不做远程协作（两人共享 tab、云同步配置）
- 不做 AI 补全 / AI 聊天
- 不做 Qt、QML、CMake、Corrosion、GPUI，或任何第二套 UI 运行时

---

## 2. 总体架构

### 2.1 分层

```
┌─────────────────────────────────────────────┐
│ src/ (React + TypeScript, repo root)        │  渲染、交互
│   └─ invoke("...")  ←  @tauri-apps/api      │
├─────────────────────────────────────────────┤
│ src-tauri/ (Rust, Tauri 2)                  │  命令桥、会话/任务状态
│   └─ 调用 pier-core::*                       │
├─────────────────────────────────────────────┤
│ pier-core/ (Rust, UI 无关)                   │  所有业务能力
│   terminal / ssh / services{git,mysql,...}  │
│   markdown / connections / credentials       │
└─────────────────────────────────────────────┘
```

**强约束**：

- `pier-core` 不得依赖 `tauri`、`react`、`gpui`、任何 UI crate。
- 前端不得绕过 Tauri 直连 `pier-core`；所有后端能力经 Tauri command。
- Tauri command 保持"薄壳"，不写业务逻辑；业务放在 `pier-core`。

### 2.2 三栏 IDE 布局

```
┌────────────────────────────────────────────────────────────┐
│                        TopBar                               │
├────────┬───────────────────────────────────────┬───────────┤
│        │  TabBar                                │           │
│        ├───────────────────────────────────────┤  Right    │
│ Sidebar│                                       │  Sidebar  │
│        │      当前 Tab 的工作区                  │  (工具    │
│ (本地  │      （终端 / Welcome）                │   面板)   │
│ 文件或 │                                       │           │
│ 服务器)│                                       │  ToolStrip│
│        │                                       │  (右侧     │
│        │                                       │   竖条)   │
├────────┴───────────────────────────────────────┴───────────┤
│                        StatusBar                            │
└────────────────────────────────────────────────────────────┘
```

### 2.3 Tab 模型

每个 tab 是"一个会话 + 一组右侧工具偏好"。

| 字段 | 含义 |
|---|---|
| `backend` | `"local"` / `"ssh"` / `"sftp"` / `"markdown"` |
| `title`, `tabColor` | 显示用 |
| `terminalSessionId` | 后端 PTY 会话 ID，为 null 表示未激活 |
| `rightTool` | 当前 tab 右侧显示哪个工具（见第 5 节） |
| SSH / Redis / MySQL / PG 等 per-service 字段 | 该 tab 上这些工具用的主机/端口/tunnel id 等 |

**`rightTool` 是 per-tab 的**，切 tab 就切右侧工具，这是 Pier-X 和"全局右侧栏"工具的核心区别。

---

## 3. 左侧 Sidebar

两个子 tab：**Files** / **Servers**。左侧不显示在 center 中打开的"项目"——center 完全由 tab 驱动。

### 3.1 Files 子 tab

- 以**家目录** (`~`) 为默认入口，而不是工作区/仓库目录。
- 支持路径面包屑、返回上一级、常用目录下拉（Home / Desktop / Documents / Downloads / Workspace）、本地搜索、刷新。
- 列表显示：名称 / 修改时间 / 大小；列头按侧栏宽度自动折叠（< 240px 隐藏修改时间，< 200px 隐藏大小）。
- **交互**：
  - 单击目录：进入。
  - 双击目录：在当前目录打开本地终端（新 tab）。
  - 单击 `.md / .markdown / .mdown / .mkdn / .mkd / .mdx` 文件：右侧自动切到 Markdown 面板并渲染该文件；选中行高亮。
  - 单击其它文件：暂无操作（预留给未来的文件预览器）。
- "Places"下拉附带"在此处打开终端"快捷项。

### 3.2 Servers 子 tab

- 显示所有已保存 SSH 连接（名称 / `user@host:port` / 认证方式徽标）。
- 支持按名称、主机、用户搜索。
- 点击任一连接：新开 SSH terminal tab。
- 每行附 **Edit** / **Delete** 两个 icon-button。
- 顶部"+"按钮打开 `NewConnectionDialog`。

### 3.3 连接持久化

- 非敏感字段（host/port/user/authKind/keyPath/name）保存在 `pier-core::connections::ConnectionStore`（YAML 文件）。
- **密码类凭证**保存在 OS keyring（macOS Keychain / Windows Credential Manager / Linux secret-service），通过 `pier-core::credentials`。
- 不把明文密码写进任何本地文件、配置或日志。

---

## 4. 中心工作区

### 4.1 TabBar

- 水平排列，支持关闭、右键菜单（Close / Close Others / Change Color）。
- 无 tab 时 center 显示 `WelcomeView`（快捷动作：新建本地终端 / 新建 SSH / 最近保存的连接 / 设置 / 命令面板）。
- Tab 颜色来自 `TAB_COLORS` 调色板（8 色），可关闭或选其中之一。

### 4.2 Terminal（当前唯一的 center 内容）

- 基于 **xterm.js** 渲染 + `pier-core::terminal::PierTerminal` 驱动（VT100 解析 + scrollback）。
- 三种后端：
  - **Local PTY**：Unix 用 forkpty，Windows 用 ConPTY。
  - **SSH shell**：`pier-core::ssh::SshSession::open_shell_channel`，russh 驱动。
  - **SSH Saved**：按 index 引用 `ConnectionStore`，自动从 keyring 拉密码。
- 支持：ANSI 颜色（256 + RGB）、粗体/下划线、光标位置、SGR、bell（可视 + 音频）、滚动 offset、可配置 scrollback 行数。
- **不支持**（明确的边界）：鼠标事件上报、Sixel/图像协议、over-SSH X11 forwarding。
- 键盘：Ctrl 组合、Meta 键、复制选中 / 粘贴剪贴板。右键自定义菜单（复制、粘贴、清屏）。
- **Tab 级生命周期**：关闭 tab 时销毁 PTY 会话、清理 tunnel。

#### 4.2.1 Smart Mode（fish 风格智能层，opt-in）

Settings → Terminal → "Smart Mode" 开关启用，**默认关闭**。开启后 Pier-X 在 PTY 之上叠一个应用层智能体验，目标对标 fish-shell 的常用功能：语法高亮、命令拼写校验、Tab 补全 popover、autosuggestion、man page 摘要弹层。

- **行边界依赖 OSC 133 prompt sentinel**：Pier-X spawn shell 时通过 `--rcfile` / `ZDOTDIR` / `$PROFILE` 注入临时 init 脚本，让用户原 PS1/PROMPT 被 `\e]133;A\a` … `\e]133;B\a` 包住；emulator 解析这两个序列得到 prompt 边界，前端在 prompt-end 后维护一份镜像 lineBuffer 做高亮与补全。**用户原 prompt 配置（git status、彩色等）不被替换**。
- **覆盖 shell**：bash、zsh、pwsh 7+；fish 检测到时直接旁路（fish 自带）。
- **自动旁路**：alt-screen 应用（vim/htop/less/tmux，由 `\e[?1049h` 触发）、bracketed paste 期间。**SSH 会话也激活**：远端 shell 不发 OSC 133 prompt sentinel，但前端镜像缓冲区在 CR/LF 上会自重置，因此 Tab popover、autosuggest、syntax highlight 都按本地终端体验提供（语法染色/灰字提示数据来自命令库 + 历史 ring，无需远端配合）。终端 header 用一枚 `Smart` pill 显示当前是否激活，旁路时改为 `Smart · idle`。
- **命令库导入**：Settings → 终端 → 命令库 提供 **Import…** 按钮（文件选择器读入 JSON），或将 importer 产出文件丢到 `app_data_dir/Pier-X/completions/packs/` 下点 Reload；用户包覆盖同名 bundled-seed。
- **键位影响**：Smart Mode 下前端拦截 Tab、↑、↓、`Ctrl+R`、`Ctrl+W`、`Ctrl+E`、`Ctrl+Shift+M` 用于补全/历史/man，其余按键仍透传给 shell readline；用户可在 Settings 里关 "Use shell-native line editing" 完全交还行编辑给 shell。
- **不持久化敏感信息**：history ring 默认仅内存；用户 opt-in 才落 `~/.pier-x/terminal-history-<shell>.jsonl`，落盘前过滤掉常见敏感模式（`*PASSWORD*`、`*TOKEN*` 行）。
- **不改默认 shell**：`default_shell()` 行为不变；Smart Mode 不内置或下载 shell 二进制。
- **Windows 限制**：M1 起 Windows 默认 `smart_mode=off`，cmd.exe 永不支持，pwsh 5 不支持，pwsh 7+ 视后续测试再决。

### 4.3 启动命令

打开 tab 时可带 `startupCommand`（例如从"在此处打开终端"进入会自动 `cd <path>`）。

---

## 5. 右侧 RightSidebar

右侧有两个组件：窄竖条 **ToolStrip**（图标按钮栏）+ 宽 **Panel** 区域。

ToolStrip 按 **6 个分类** 分组渲染，相邻分类之间插入一条细分隔线（不显示文字标签——竖条太窄；分类语义靠 hover tooltip 显示）。每个工具的 `category` 字段固定，前端根据 `RIGHT_TOOL_ORDER` 顺序遍历，category 变化处自动加 divider。

| 分类 | 含义 | 工具 | 图标 |
|---|---|---|---|
| **workspace** | 工作区（本地文件 / 当前目录的工程操作） | Markdown / Git | FileText / GitBranch |
| **host** | 主机概览（read-mostly 的 OS 级视图） | Server Monitor / Firewall | ChartNoAxesCombined / Shield |
| **files** | 文件与日志 | SFTP / Log | FolderSync / LogIcon |
| **containers** | 容器 | Docker | DockerIcon |
| **database** | 数据库 | MySQL / PostgreSQL / Redis / SQLite | MySqlIcon / PostgresIcon / RedisIcon / SqliteIcon |
| **service** | 主机一级服务管理 | Web Server / Software | Globe / Package |

完整渲染顺序（工具栏第一位就是默认工具）：

| # | 工具 | 分类 | 作用域 | 远程必需？ |
|---|---|---|---|---|
| 1 | **Markdown** | workspace | 预览当前选中的本地 .md（来自左侧 Sidebar） | — |
| 2 | **Git** | workspace | 对当前浏览路径（`browserPath`）做 Git 操作 | — |
| 3 | **Server Monitor** | host | 远程主机状态快照 | 需 SSH tab |
| 4 | **Firewall** | host | 防火墙规则 / 监听端口 / 接口流量 / 端口映射 | 需 SSH tab |
| 5 | **SFTP** | files | 远程文件浏览/上传/下载 | **仅** SSH tab |
| 6 | **Log** | files | 流式查看远程命令输出 | 需 SSH tab |
| 7 | **Docker** | containers | 本地或远程 Docker 管理 | 支持两种模式 |
| 8 | **MySQL** | database | 通过 SSH tunnel 到远程 MySQL，或本地 | 需 tab |
| 9 | **PostgreSQL** | database | 同上 | 需 tab |
| 10 | **Redis** | database | 同上 | 需 tab |
| 11 | **SQLite** | database | 打开本地 `.db` 文件（local-only，仍归入 database 分类） | — |
| 12 | **Web Server** | service | 远端 web 服务器（nginx / Apache / Caddy）一站式管理 | 需 SSH tab |
| 13 | **Software** | service | 远端工具栈一览 / 安装 / 更新 / 启用服务 | 需 SSH tab |

**为什么按这个顺序排分类**：从最"贴身"的工作区往外铺开——本地文件 → 主机概览 → 远端文件 → 容器 → 数据库 → 服务。日常用最多的（Markdown / Git / Monitor）固定在最上面，配置类的（Web Server / Software）放在最末尾，和"主机一级运维"语义对齐。

Web Server 是一个**统一入口**，不是单一产品面板：进入后通过 SSH 探测 host 上实际安装的 web server（nginx / apache / caddy），单装时直接路由到对应面板，多装时显示顶部 segmented control 切换。详见 §5.12。

**默认 `rightTool`**：
- 本地 tab / 无 tab（欢迎页）：`markdown`
- SSH tab：`monitor`

### 5.1 Markdown 面板

- **输入**：左侧 Sidebar 选中的 `.md` 文件路径（`selectedMarkdownPath`）。
- **渲染**：Tauri 命令 `markdown_render_file(path)`，后端用 `pulldown-cmark`（CommonMark + GFM）。
- **状态**：未选 → 提示"在左侧选择 Markdown 文件"；加载中 → "渲染中…"；错误 → 红色错误文本；成功 → HTML 预览。
- **不含**：原地编辑、外链图片代理、自动刷新监听文件变化（未来项）。

### 5.2 Git 面板

- **作用范围**：左侧 Sidebar 的 `browserPath`（当前浏览目录）。如果不是 git 仓库，面板允许"初始化"。
- **总览**：分支、tracking、ahead/behind、staged/unstaged 数量、变更列表。
- **操作**：暂存 / 取消暂存 / 丢弃（需确认）/ 提交 / 提交并推送 / 推 / 拉 / fetch。
- **分支**：列表、切换、创建、重命名、删除、跟踪设置、删除远程分支。
- **历史**：提交图（`git_graph`），点击查看 commit 详情（作者、日期、stats、改动文件列表）、文件级 diff、blame。
- **Stash**：列表、push、apply、pop、drop。
- **Tags**：列表、创建、推送单个 / 推送全部、删除。
- **Remotes**：列表、新增、修改 URL、删除、fetch。
- **Config**：读取 + 修改（local / global）。
- **Rebase**：交互式 rebase 计划、执行、abort、continue。
- **Submodules**：列出、init、update（递归）、sync。
- **Conflicts**：列出冲突文件、按整文件接受 ours/theirs、逐 hunk 标记解决。
- **右键菜单**：变更行的暂存 / 取消暂存 / 丢弃 / 查看 diff / blame。

Git 面板是功能最密集的面板，视觉上享有"无标题栏"的特例（`right-sidebar__content--git`），以让出垂直空间。

### 5.3 Server Monitor 面板

- SSH tab 专属（local terminal tab 也可用作"本地监控"）。
- 显示：uptime、load (1/5/15)、内存/swap、磁盘（聚合 + 每挂载明细 + 块设备拓扑）、CPU%、网络吞吐、Top 进程（按 CPU/内存切换）。
- 命令：`server_monitor_probe`（SSH） / `local_system_info`（本地）。两条命令都接收 `include_disks: bool`：
  - `false`：fast tier，只跑 `uptime` / `free` / `/proc/stat` / `/proc/net/dev` / `ps`。
  - `true`：full tier，额外执行 `df -hPT` 与 `lsblk -P -b -o NAME,KNAME,PKNAME,TYPE,SIZE,ROTA,TRAN,MODEL,FSTYPE,MOUNTPOINT`。
- 自动轮询节奏：
  - 5 s 一次 fast probe；每隔 30 s 该 tick 升级为 full probe。
  - 用户点 "立即探测" 按钮始终触发 full probe。
  - 面板隐藏（切到其它工具）时整套轮询暂停，避免 keep-alive 实例后台烧 SSH。
  - 上一次 full probe 的磁盘字段 (`disks` / `blockDevices` / 顶部聚合 `disk_*`) 在 fast tick 之间被前端保留并继续渲染，避免闪烁。
- 顶部"磁盘" gauge 与 pill 语义：**所有可见挂载求和**（`disk_total` = Σ total，`disk_use_pct` = Σ used / Σ total）。被过滤掉的伪文件系统、Docker overlay、snap 挂载等不参与求和。`/` 单挂载主机的读数与原行为一致。
- 块设备子区（`BLOCK DEVICES`）渲染 `lsblk` 树状关系：物理盘 → 分区 → crypt/LUKS → LVM → 挂载点。每个物理盘行展示介质类型（SSD/HDD，来自 ROTA）与传输总线（NVMe/SATA/virtio/USB，来自 TRAN）；MODEL 字符串放在 row tooltip。lsblk 不可用（macOS 本地、BusyBox 远端）时该子区整体隐藏，DISKS 表与顶部聚合仍按 `df` 数据正常工作。

### 5.4 Docker 面板

- 双模式：
  - **本地**：调用 `local_docker_overview` / `local_docker_action`，不需要 SSH。
  - **远程**：通过 SSH session 执行 `docker ...` 命令，解析为结构化数据。
- 五类资源 tab：Containers / Images / Volumes / Networks / **Projects**。
- 操作：容器 start/stop/restart/remove、inspect（JSON 弹窗）、镜像/卷/网络删除（force 选项）。
- **Projects（Compose 项目视图）**：
  - 按 `com.docker.compose.project` 标签对容器分组，项目下列出 `com.docker.compose.service` 服务名及对应容器状态（running / exited / ...）。
  - 纯派生视图：不引入 `docker-compose` / `docker compose` 子进程，也不需要读取 compose YAML。所有信息来自已有的 `docker ps` 标签输出。
  - 无 compose 标签的容器不出现在该 tab（它们只在 Containers tab 里显示）。
  - 操作复用 Containers tab 的单容器动作；**不**提供"整个项目 up/down"，因为那需要 compose CLI 的声明式模型，超出本面板"直接控制容器"的定位。
- **远端未装 Docker 时**：面板 inline 渲染"安装 Docker"CTA，复用 §5.11 软件注册表（packageId=`docker`，`enableService=true`）+ 流式输出；安装完成后自动 `refresh()`。

### 5.5 MySQL / PostgreSQL 面板

- **连接建立**：
  - 如 SSH tab：自动开 SSH tunnel（`ssh_tunnel_open`）到远程 3306 / 5432，记录 `mysqlTunnelId` / `pgTunnelId` / `mysqlTunnelPort` / `pgTunnelPort` 到 tab state，连接走 `127.0.0.1:<localPort>`。
  - 本地直连也支持（填本地 host / port）。
- **浏览**：database / schema / table 三级选择器；表列 metadata 展示，含 `column_comment` / `table_comment`（MySQL 走 `SHOW FULL COLUMNS` + `information_schema.tables.table_comment`，PG 走 `col_description` / `obj_description`）。
- **数据预览**：`SELECT * FROM <table> LIMIT N` 结果表（`PreviewTable`）。
- **查询编辑器**：原生 SQL 输入 + 执行（`mysql_execute` / `postgres_execute`）。
- **结果集**：`QueryResultPanel` 显示列、行、耗时、截断标记、影响行数、last insert id；支持导出为 TSV（`queryResultToTsv`）粘贴到表格软件。
- **结构 Tab**：
  - 列表渲染 name / type / null / default / extra / **comment**（MySQL+PG 渲染 comment 列，SQLite 隐藏）。
  - 在写入解锁状态下，**name / type / comment 均支持单元格内联编辑**，连同 add / drop column 一起 batch 提交。`type` 在 SQLite 上禁用（无 in-place type change）。MySQL 的 `MODIFY COLUMN` 会自动注入原列的 nullable / default / comment 快照以保留无关字段。
- **Schema Tab（数据库变量 / settings / pragmas）**：
  - 在写入解锁状态下，**值列支持点击内联编辑**：MySQL 走 `SET GLOBAL`（read-only / non-dynamic 变量保持只读），PG 走 `ALTER SYSTEM ... + SELECT pg_reload_conf()`（`postmaster` / `internal` context 的设置只读，UI 提示需重启），SQLite 走 PRAGMA 白名单（journal_mode / synchronous / cache_size / foreign_keys / temp_store / wal_autocheckpoint / busy_timeout / user_version）。
  - 不可写行渲染锁图标 + 引擎报告的 hint（"global only" / "requires restart (postmaster)" / "connection-scoped"）。
  - 编辑失败回滚到原值并把引擎错误就地展示在行下方。
- **宽区 Schema Tree 右键菜单**：
  - 树根 / 数据库行：`新建库…`（MySQL/PG，弹 `DbCreateDbDialog`；SQLite 不出现，因为 SQLite 没有"库"概念，用文件保存对话框新建 `.db`）/ `Import SQL…`（OS 文件选择 → `local_read_text_file` → `splitSqlStatements` 拆分 → 逐条 execute）/ `Drop database…`。
  - 表行（支持 `cmd/ctrl + click` 多选 / `shift + click` 范围选）：`Copy name` / `Export…`（OS 保存对话框 → `exportTablesAsInserts` 生成 INSERT 序列 → `local_write_text_file`，本期为**仅数据**导出，每张表 5 万行硬上限，超限会在通知里点出哪张表被截断）/ `Truncate…` / `Drop table…`。
  - 鼠标悬停表行的 tooltip 含 `engine · data X · idx Y · updated Z · comment: …`。
- **导出当前局限（已知）**：上面那条 INSERT-only 路径**不**导出 CREATE TABLE / 索引 / 触发器 / 外键 / routines。整库一致性快照仍需 `mysqldump` / `pg_dump` / `sqlite3 .dump` 二进制集成 — 该后端命令族留待下一轮迭代。
- **安全模式（关键）**：
  - 默认只允许以 `SELECT / SHOW / DESCRIBE / EXPLAIN / PRAGMA / USE / SET / BEGIN / COMMIT / ROLLBACK` 等只读关键字开头的语句（`isReadOnlySql`）。
  - 写操作需要**显式解锁写入**（UI 开关），并在执行前再次确认。绝不能"智能识别无害 DELETE"。
  - 这个约束未来不能被放宽。
  - Schema Tab 的"值"内联编辑、Structure Tab 的列编辑、右键菜单中的破坏性动作（Truncate / Drop / Import-execute）**全部受同一只读开关控制** — 解锁前菜单项隐藏（`onTruncateTables` 等回调在 `readOnly` 时为 `undefined`），Drop / Truncate 解锁后还要走 `ConfirmDialog` 二次确认。
- **远端未装 mysql / psql 客户端时**：splash 在 `extraBody` 渲染 inline 安装 CTA（packageId=`mariadb` / `postgres`，`enableService=false`，避免在 SSH 主机上意外暴露新启动的 daemon）；安装完成后调用 `flow.refreshDetection()`。

### 5.6 SQLite 面板

- 本地文件；通过"选择 .db 文件"输入路径。
- 表列表 / 列 metadata / 预览 / 查询同 MySQL 逻辑。
- 同样的只读默认 + 显式解锁规则。
- **结构 Tab 与 MySQL/PG 不同的地方**：comment 列**整列隐藏**（`commentEditable={false}`，传 `dialect="sqlite"` 让 type 列也禁用 in-place 修改 — 提示 "SQLite does not support changing column types in place"）。其他列编辑（rename / add / drop）照常工作。
- **Schema Tab**：PRAGMA 白名单可编辑；编辑落到的 PRAGMA 是 connection-scoped（per-connection），UI 在每行的 tooltip 里点出这一点。
- **右键菜单**：与 MySQL / PG 共用 `DbSchemaActions` 接线，缺 `onCreateDatabase`（SQLite 的"库"=文件，用 splash 的文件选择器创建）。其余项一致：Copy / Refresh / Truncate（走 `DELETE FROM` — SQLite 没有 TRUNCATE，依赖 truncate optimisation）/ Drop / Import / Export。
- 远端缺 sqlite3 时的"自动检测并安装 sqlite3"按钮现走 §5.11 软件注册表（packageId=`sqlite3`，流式输出），与其他面板共享同一安装路径；安装完成后再次 probe `sqliteRemoteCapable` 以拿到 `supportsJson` / 版本。

### 5.7 Redis 面板

- 连接：同样支持 SSH tunnel。
- **扫描**：pattern (默认 `*`) + limit；超限截断提示。
- **key 详情**：type（string/list/hash/set/zset/stream）、TTL、编码、首若干成员预览。
- **命令编辑器**：空格分隔的 Redis 命令（例如 `SET foo bar`），返回摘要 + 多行输出 + 耗时。
- **危险动作**：`FLUSHDB` / `FLUSHALL` / `KEYS *`（大库阻塞）必须给醒目警告；UI 不禁用但要求二次确认。
- **远端未装 redis-server 时**：splash 在 `extraBody` 渲染 inline "安装 Redis" CTA（packageId=`redis`，`enableService=false`）；安装完成后调用 `refreshDetection()` 重扫服务。

### 5.8 SFTP 面板

- **仅** SSH tab 可用。
- 远程路径栏、文件列表（名称 / 大小 / 权限 / 类型）。
- 操作：mkdir / rename / remove / download（写到本地指定路径）/ upload（从本地选文件）。
- **右键菜单**：文件行上提供 Open/Edit/Download/Rename/Duplicate/Delete/Change permissions/Copy path/Properties；空白区域提供 New file/New folder/Upload/Refresh。右键总是先选中当前行以对齐行为。
- **New file**：在当前目录下通过 `sftp_create_file` 创建空文件；与 `mkdir` quickrow 同构的内联输入行。
- **Change permissions (chmod)**：弹出权限编辑对话框，owner/group/other × r/w/x 勾选 + 八进制直接编辑 + `rwxrwxrwx` 即时预览，提交后调用 `sftp_chmod`。后端用 `russh-sftp` 的 `set_metadata` 设置 mode & 0o7777。
- **内嵌编辑器**：对可识别文本扩展名（`.conf`/`.sh`/`.json`/`.yaml`/`.ts`/`.py`/`.env` 等）且 ≤ 5 MB 的文件，双击或右键 Edit 打开。编辑器基于 CodeMirror 6，包含 Ctrl+F 查找 / Ctrl+H 替换 / 正则 / 矩形（列）选择 / 括号匹配 / 代码折叠 / 语法高亮；主题走 `var(--*)` 令牌，跟随 pier-x 主题切换。Ctrl+S 保存，脏标记显示在标题栏；Esc 关闭（若脏会二次确认）。
- **非 UTF-8 保护**：后端读取文件时用 `from_utf8_lossy` 替换非法字节为 U+FFFD 并在响应里携带 `lossy: true`。编辑器显示警告条，提醒用户保存会持久化替换结果。同时后端对读文件做 5 MB 硬上限，超限拒绝，避免编辑器吞巨型日志。
- **Duplicate**（仅文件）：用 read_text + write_text 做服务器侧"复制为 副本"；同样受 5 MB 限制，超限要求用户改走下载再上传路径。
- 大文件上传/下载走 `sftp:progress` 事件流，传输队列显示活动/完成数量和进度百分比。

### 5.9 Firewall 面板

- **SSH tab 专属**。后端类型自动探测：检测顺序 `firewall-cmd` → `ufw` → `nft list ruleset` → `iptables-save`，第一个能在当前主机用的就是 backend。展示在面板头部（"backend: iptables-nft (root)"）。

- **位置**：右侧工具栏，紧随 Monitor 之后——防火墙（端口暴露 / 命中计数 / 接口流量）和 Monitor（CPU / 内存 / 进程）都是只读为主的主机概览，归一类。

- **数据源全部使用基础工具，零额外安装**：
  - 规则：`iptables-save` / `nft -j list ruleset` / `firewall-cmd --list-all-zones` / `ufw status verbose`
  - 监听端口 + 进程：`ss -tulnpH`
  - 接口流量：每 2s 采样 `/proc/net/dev`，做差分 → 字节速率
  - 端口映射 / NAT：`iptables -t nat -S`（含 Docker 注入的 DOCKER 链）

- **Tab 划分**：
  - **Listening**：所有 TCP/UDP 监听 socket，列：port / proto / process / pid / bind addr。每行带 "Block" 按钮。
  - **Rules**：当前 backend 的 INPUT / OUTPUT / FORWARD 链，按链卡片化展示，命中计数可见。每行 "Delete"。
  - **Mappings**：DNAT / Docker `DOCKER` 链的端口转发规则。
  - **Traffic**：按接口的 RX/TX 字节速率 sparkline（5 分钟窗口，2s 步长）。

- **写操作策略 — 走终端通道，不静默执行**：
  - 所有可写动作（Block / Allow / Delete rule）都通过 `terminal_write` 把命令注入到该 tab 的终端，**不带尾部回车**，由用户自己审阅 + 按 Enter + 输入 sudo 密码。
  - 命令模板按探测到的 backend 切换：iptables 用 `-A INPUT ... -j ACCEPT`、ufw 用 `ufw allow NN/tcp`、firewalld 用 `firewall-cmd --add-port=NN/tcp --permanent`、nft 用 `nft add rule ...`。
  - 不持有也不传输 sudo 密码；面板没有"输入密码"输入框。
  - 单页面只能写命令到当前 tab 的终端，没有终端的 tab（如纯本地无终端会话）禁用所有写操作并提示。

- **不做**：自动应用 `iptables-save` 持久化、规则可视化拓扑图、规则模板向导、IPv6 单独 tab（IPv4/v6 在同一视图按 family 列展示）。

### 5.10 Log 面板

- SSH tab 专属。
- **日志源（LogSource）** 通过结构化选择而非裸命令决定 —— 前端把选择编译成一条 shell 命令后再走 `log_stream_start`。三种模式：
  - **File**：给定远端目录路径，列出该目录下常见日志文件（`.log` / `.out` / `.err` / `.txt`），选一个即编译为 `tail -F <path>`。目录列表复用已有 `sftp_browse`，不引入新后端命令。
  - **System**：一组预设命令，覆盖典型系统日志源：
    - `syslog` → `tail -F /var/log/syslog`
    - `auth.log` → `tail -F /var/log/auth.log`
    - `nginx access / error` → `tail -F /var/log/nginx/access.log` 等
    - `dmesg` → `dmesg -w`
    - `journald (all)` → `journalctl -f`
    - `journald unit` → `journalctl -u <unit> -f`（需填 unit 名）
    - `docker container` → `docker logs -f <container>`（需填容器名/id）
  - **Custom**：仍允许自定义命令字符串，作为 `⋯` 二级入口，不是默认入口。
- 选择态持久化在 `TabState.logSource`；`logCommand` 字段保留用于兼容和调试显示。
- 后端仍只暴露 `log_stream_start / log_stream_drain / log_stream_stop` 三条命令。前端轮询 drain 事件。
- 不是"实时 tail"，是"前端按需 drain"模型，避免 Tauri 事件风暴。
- 视觉对齐 pier-x (Remix) 参考稿：命令字符串不再作为主要入口暴露，默认展示"源摘要 + 流状态 + Start/Stop"一行，下方用与 db-picker 同构的选择器行，避免让终端用户直接编辑 shell 命令。

### 5.11 Software 面板

- **SSH tab 专属**。展示当前主机的工具栈安装情况，提供一键安装 / 一键更新；输出**流式**回显。
- **数据源**：纯远端命令（`/etc/os-release`、`command -v`、`<bin> --version`、`systemctl is-active`、对应包管理器），不依赖额外安装。
- **入口位置**：右侧 ToolStrip 末位。和具体业务工具（Docker / DB / SFTP / Firewall）区分，定位为"主机一级"运维。
- **支持的发行版**：Debian / Ubuntu / Mint / Raspbian / Pop / Elementary / Kali（apt）、Fedora / RHEL / CentOS / Rocky / Alma / OL / Amazon（dnf 优先回落 yum）、Alpine（apk）、Arch / Manjaro / EndeavourOS（pacman）、openSUSE / SLES（zypper）。未识别发行版面板可以列状态但安装按钮禁用。
- **v1 注册表**（9 项）：sqlite3 / docker / docker-compose / redis / postgresql / mariadb（提供 mysql 客户端） / nginx / jq / curl。每项包含：probe 命令、按包管理器映射的包名列表、可选 systemd 服务名。
- **行渲染**：
  - 状态图标：✓ 已装 / ◯ 未装 / ⏳ 进行中
  - 已装时显示版本（来自 probe 输出）
  - 主操作按钮：未装 → "安装"；已装 → "更新"
  - 服务字段（若有）：装完自动 `systemctl enable --now`，UI 上有个可关闭的 "启用并启动服务" 总开关
- **流式输出**：点 [安装] / [更新] 后行下展开 max-height 220px 的滚动 log 框，每读到一行远端 stdout/stderr 就追加；结束态摘要：`已装 vXX via apt` / `失败 (exit 100)` 等。
- **并发约束**：同主机同一时刻只允许一个安装/更新；其他行按钮 disabled。多主机互不影响。
- **包源**：仅发行版默认仓库（apt / dnf / yum / apk / pacman / zypper）。**不**走 get.docker.com 等官方脚本，**不**自动加第三方仓库 / GPG key。
- **版本**：v1 仅安装/更新到发行版默认版本。
- **sudo**：非 root 时命令前加 `sudo -n`；密码场景识别为 `sudo-requires-password`，给出"请用 root 登录或配置免密 sudo"的明确提示。
- **复用**：SQLite 面板的"自动安装 sqlite3"按钮内部走同一注册表 / 同一流程；不重复实现发行版识别。

#### v1.1 — 卸载

- 每行的「安装 / 更新」按钮旁紧邻一个 ⋯ 菜单，菜单项目前只有 **「卸载」**（未安装时禁用并附"安装后才能卸载"提示）。点击打开**卸载对话框**。
- 卸载对话框三个独立勾选：
  - **同时移除配置**：apt → `purge`；pacman → 加 `-n`；其它包管理器原本就清配置，此选项无副作用。
  - **同时清理依赖**：apt → 紧接 `apt-get autoremove -y`；dnf/yum → 同等 `autoremove`；zypper → 切换为 `--clean-deps`；pacman → 切换到 `-Rs`（与 purge 选项组合得到 `-Rns`）；apk 无对应概念，silently 忽略。
  - **同时删除数据目录（不可恢复）**：仅当 descriptor 声明了 `data_dirs`（docker → `/var/lib/docker`、`/var/lib/containerd`；postgres → `/var/lib/postgresql`、`/var/lib/pgsql`；redis → `/var/lib/redis`；mariadb → `/var/lib/mysql`）时才出现；勾选后必须额外输入软件 id 二次确认才能解锁红色「卸载」按钮。脚本里这一步用 `&&` 串接在包管理器 remove 之后，remove 失败不会执行 `rm -rf`。
- **服务处理**：descriptor 声明了 `service_units` 的软件，卸载脚本前缀一段 `command -v systemctl >/dev/null 2>&1 && systemctl disable --now <unit>`，以 `;` 收尾——失败、无 systemd（如 alpine）都不中断后续 remove。
- **No-op 快路径**：开始前先 probe；未安装直接返回 `not-installed` 状态（不发任何包管理器命令）。
- **事件通道**：卸载用独立的 `software-uninstall` 频道（payload 形状与 `software-install` 同构但 report 字段不同——多出 `dataDirsRemoved`、不带 `version` / `serviceActive`）。前端按 `installId` 过滤，多面板并发不串扰。
- **不做（v1.1）**：版本回滚、批量卸载、镜像源切换、保留某些数据子目录的精细控制、卸载结束后再启动相邻服务（典型如 docker 卸了之后是否要启动备用 runtime——这是 ops 决策不该自动化）。

#### v2 — 版本选择器

- 每行的「安装 / 更新」按钮变成 split-button：左半保持原行为（装/更新到发行版仓库当前版本），右半 ▼ 打开版本下拉。
- 版本枚举命令按包管理器分发：apt → `apt-cache madison`；dnf / yum → `list available --showduplicates`；apk → `apk version -a`；zypper → `zypper search -s`；**pacman 不支持**（Arch 标准仓库只有最新版），因此 pacman 主机上不显示 ▼。
- 版本顺序保持包管理器原始输出（通常最新版在最上方），前端去重不重排。
- 选定具体版本后，命令拼接：apt / apk / zypper → `<pkg>=<version>`；dnf / yum → `<pkg>-<version>`；pacman 即使透出 version 也忽略（防御性兜底）。
- 缓存：每个 host+package 的版本列表本地缓存 5 分钟，dropdown 二次打开秒开，不重发远端命令。
- 已装软件如果列表里第一条 ≠ 当前装的版本（即有更新可用），打开 dropdown 时 [更新] 按钮默认预选最新版（按钮文案变 "更新到 v3.45.1"）；用户选 "Latest" 可清除选择。
- 不做：版本格式校验（让包管理器报错就行）、降级保护（最多在按钮旁提示一句"将降级到 v…"）、debian alternatives 多版本共存切换。
#### v2 — 服务控制 + 日志查看

- 行的 ⋯ 菜单在「卸载」之上扩展四项（仅当 descriptor 声明了 `service_units` **且**当前已安装时才出现，未装一律不显示，hairline divider 与下方「卸载」分隔）：
  - **重启服务**：`systemctl restart <unit>`，掉连接但配置生效——多数面向连接的 daemon（redis / postgres / mariadb / docker）改完配置走这条。
  - **停止服务** / **启动服务**：根据当前 `serviceActive` 二选一显示——状态为 `false` 时显示「启动」，否则显示「停止」。避免菜单里同时有两条容易误触。
  - **热重载（不中断连接）**：仅当 descriptor 声明 `supports_reload: true` 时出现（v2 注册表里目前只有 nginx）。映射到 `systemctl reload <unit>`。Apache 等多 worker 模型的 daemon 后续如果加进注册表就开 `supports_reload`，其它 daemon 的 reload 等价于 restart，故意不暴露以避免误导。
  - **查看日志**：在面板内开一个 `<Dialog>`（size=lg），首次打开自动拉取 `journalctl -u <unit> -n 200 --no-pager 2>&1`，dialog 自带「Refresh」按钮重拉。**不做实时 tail**——cancel 语义复杂、与现有 Log 面板职能重复，留给 v3。
- **服务状态点**：行头版本号旁边显示一个 `<StatusDot tone="pos|neg" />`——绿（active）/ 红（inactive）。`serviceActive === null`（descriptor 无 service 或 systemctl 缺失）时不渲染。点本身用 `title` 提示中文「服务已启动 / 服务已停止」，不写文字旁注以避免行尾过挤。
- **后端**：`pier-core::services::package_manager::service_action(session, descriptor, action, on_line) -> ServiceActionReport`，按 `descriptor_service_unit(descriptor, manager)` 解析 unit，非 root 加 `sudo -n`。报告含 `status: ok | sudo-requires-password | failed` 三态、`exit_code`、`output_tail`、以及关键的 `service_active_after`（动作完成后再跑一次 `systemctl is-active` 作为 ground truth——`exit 0` 但 unit `failed` 也算 `failed`）。`journalctl_tail` 是单次 `exec_command`，不走流式。
- **事件通道**：服务动作走独立的 `software-service-action` 频道，payload 形状沿用 `kind: "line" | "done" | "failed"`，但 `report` 是 `ServiceActionReportView`（含 `action` / `unit` / `serviceActiveAfter`，不含 install/uninstall 专有字段）。日志拉取 `software_service_logs_remote` 是同步返回 `Vec<String>`，不发事件。
- **store 模型**：`SoftwareActivity::kind` 在 v1 的 `install / update / uninstall` 三态基础上加 `service-start / service-stop / service-restart / service-reload` 四态，触发同一行 busy 互斥（同主机同时刻只跑一个动作，install 和 service action 之间也互斥——同行的「安装中」按钮自然 disabled，但「⋯」菜单也一并 disable 防止用户在装 redis 时去 stop redis）。
- **状态翻转**：动作完成后**不**做整体 re-probe，仅用 `report.service_active_after` 翻转该行的 dot——多数情况服务状态变化只影响发起动作的那一行，省一次全量 probe。
- **sudo 失败**：`sudo-requires-password` 路径走与 install / uninstall 同样的中文提示（"sudo 需要密码——请用 root 登录或配置免密 sudo"），不在 UI 里要密码。
- **不做（v2）**：实时 tail（已有 Log 面板）、systemd timer 管理、依赖图可视化、unit 的 environment vars 编辑、按 timestamp / 日志级别过滤的 journalctl 视图。
#### v2 — 取消（install / update / uninstall）

- 行 busy 时主操作按钮替换成红色「取消」按钮（同 `btn is-danger is-compact`）；点击后立刻 disable 并显示「取消中…」，等收到终态事件再恢复。同主机其他行的安装按钮在取消生效后立即可用——「同主机仅一个 in-flight」的 v1 约束保持不变。
- **后端实现**：`pier-core::ssh::SshSession::exec_command_streaming` 新增 `cancel: Option<CancellationToken>` 形参，触发后通过 `tokio::select!` 关闭 channel 并返回 `exit_code = -2`（`CANCELLED_EXIT_CODE`）。`package_manager::install / update / uninstall` 透传 token，命中后报告 `status = "cancelled"`，**跳过 post-install 重新 probe / `systemctl enable --now` / 数据目录 `rm -rf`** ——状态以下一次手动 probe 为准。
- **事件通道**：`software_install_cancel(installId)` 命令查表 → trigger token → 在 `software-install` 与 `software-uninstall` 两个频道各 emit 一条 `kind: "cancelled"`，再从注册表清掉。前端 listener 按 `installId` 过滤；同时收到 `done` + `cancelled` 时**`cancelled` 优先**。
- **远端进程不保证停**：取消只让本端不再读取输出，远端 apt / dpkg / dnf 进程**可能继续跑完**，并留下 `dpkg --configure -a` 之类的 lock 状态。UI 提示文案（`Cancel signal sent — the remote may still be running.`）说清楚这是 ops 责任、Pier-X 不做自动清理。
- **不做（v2）**：远端进程真正杀干净（apt 的 dpkg lock / apk 的 apk-tools.lock）、取消正在 `systemctl enable` 的服务、取消进行到一半的 `rm -rf data_dirs`（数据已被部分删除时不应自动回滚）。
#### v2 — 官方脚本通道（vendor_script）

- **目的**：发行版仓库里的版本对部分软件来说滞后太多（典型例：Ubuntu LTS 的 `docker.io` 通常落后上游 docker-ce 一年以上）。v2 引入"官方脚本通道"作为**第二条**安装路径，**不替换**默认 apt / dnf / … 路径。
- **支持的软件（v2）**：仅 **docker** 一项，URL 写死为 `https://get.docker.com`。其它软件（postgres / node 等）的 vendor_script 字段保留为 `None`，扩展时再补；新增 vendor 源**必须**作为代码改动落到 `pier-core::services::package_manager` 注册表里——前端无任何用户输入 URL 的入口。
- **UI 形态**：descriptor 声明了 `vendor_script` 时，[安装] 按钮变 split-button：主按钮直接走默认 apt 通道；右侧 ▼ 点开两项菜单："通过 apt（默认）" / "通过 {label}"。已安装的行只显示 [更新] 按钮（不带 ▼）——v2 通道**仅安装**，更新沿用原 apt 路径。
- **二次确认**：选官方脚本时弹出确认对话框：
  - 显示脚本 URL（可点 📋 复制）。
  - 显示维护方说明（descriptor 的 `notes` 字段）。
  - `conflicts_with_apt` 为 true 时显示警告："此官方脚本与发行版仓库的包可能冲突，如已通过 apt 安装请先卸载"。
  - 必勾："我了解 Pier-X 不会校验脚本签名"。
  - 默认按钮：[取消]（焦点在这）；主操作按钮：红色 [继续安装]，未勾时禁用。
- **远端流程**（pier-core::package_manager::install_via_script）：
  1. `curl -fsSL '<url>' -o /tmp/pier-x-installer-{package_id}.sh`（不需要 root）。
  2. `stat` 校验文件非空（防 200-空 body / 透明代理占位）。
  3. `sh /tmp/pier-x-installer-{package_id}.sh`（vendor.run_as_root=true 时加 `sudo -n` 前缀）。
  4. `trap 'rm -f …' EXIT` 保证临时文件总会被清理；下载失败 / sudo 失败时再补一次 `rm -f` 兜底。
  5. 装完跑 `package_manager::probe_status` 确认 binary 在 PATH。
  6. **不**用 `bash -c "$(curl ...)"`，**不**用 `curl … | sh`：分两步（download → exec）便于 rollback / 调试 / 短路（步骤 2 的大小检查）。
- **失败分类**：v2 引入两个新 InstallStatus
  - `vendor-script-download-failed`：curl 退出非 0（DNS / 网络 / TLS / 4xx / 空 body）。
  - `vendor-script-failed`：脚本退出非 0 且 post-probe 仍找不到 binary。
- **报告字段**：`InstallReport.vendor_script` 为 `Some({label, url})` 时，前端在日志末尾追加一行 `via {label}（{url}）`，作为审计痕迹。
- **风险声明**：v2 **不做**任何签名 / 哈希校验——脚本下载后直接执行。这一事实在确认对话框里以"我了解…"勾选项的形式向用户明示。GPG / sha256 校验需要建立公钥仓 / hash 注册表，留给 v3。
- **§1.2 不做的事**不松绑：脚本仍在远端 `sh` 里跑，仍是 shell 流程，没有引入 web 运行时 / 浏览器嵌入 / 任意 URL 抓取。攻击面 = 注册表里写死的那一条 URL 的供应链。
- **不做（v2）**：
  - GPG / sha256 校验（v3）。
  - 自动加第三方仓库（`add-apt-repository docker-ce` 等）——跟 vendor 脚本是两条互斥路线，不混用。
  - 脚本中途的进度条解析（apt 一样不做）。
  - 多 URL 选择（同一软件多个上游）。
  - Update 走 vendor_script 路径——上游脚本是安装器不是 upgrader，混用容易破坏 apt repo 状态。

### 5.12 Web Server 面板（nginx / Apache / Caddy）

- **SSH tab 专属**。一面板覆盖"找到配置文件 → 结构化编辑 → 校验 → reload → 站点 toggle"全流程，不走 SFTP，不走外部编辑器。
- **位置**：右侧 ToolStrip 紧邻 Software 之前。两者都是"主机一级"运维工具——配置编辑可能引出"装个 nginx-extras / mod_security"的需求，相邻摆放方便用户串联。
- **统一入口架构**：`WebServerPanel` 是顶层路由组件，挂载后先调一次 `web_server_detect`（探测 nginx / httpd / apache2 / caddy 的存在与版本）。
  - 单装时自动路由到对应的子面板，segmented control 隐藏，UX 与单产品一致。
  - 多装时顶部出现 segmented control（nginx · Apache · Caddy），运行状态用 systemd `is-active` 着色（绿点 active / 红点 inactive）。
  - 都没装时显示空状态卡片 + "去 Software 面板装一个"的引导。
  - 持久化保存的旧 `rightTool: "nginx"` 在 `useTabStore.scrubRuntimeFields` 里自动迁移为 `"webserver"`。

#### 5.12.1 nginx 子面板（最深的功能集，22 项 feature catalog）
- **数据源 / 命令**（全部在远端执行）：
  - 探测：`command -v nginx && nginx -v 2>&1` / `nginx -V 2>&1` / `id -u`
  - 文件树：`find /etc/nginx/conf.d -name '*.conf'` / `find /etc/nginx/sites-available` / `find /etc/nginx/sites-enabled -type l` + `readlink -f`
  - 读：`cat <path>`（最大不限，nginx 配置一般 KB 级；非 UTF-8 字节走 `from_utf8_lossy`）
  - 写：`base64 -d > /tmp/...` → `chmod --reference=<原> /tmp/...` → `mv /tmp/... <原>`（原子覆盖）
  - 校验：`nginx -t`
  - Reload：`systemctl reload nginx`，systemctl 缺失时回落 `nginx -s reload`
  - 软链接 toggle：`ln -sf` / `rm -f`
- **解析器**：`pier-core::services::nginx::parse` / `render`，纯 Rust 手写，**不**引入 nom 或第三方 crate。AST 节点保留前导注释、空行计数、行内注释，能 round-trip。`*_by_lua_block` / `*_by_njs_block` 的 body 作为 opaque blob 保留——结构化卡片中只读，编辑请切到「原文」模式。
- **文件树**（左栏）：按 `nginx.conf` / `conf.d` / `sites-available` / `sites-enabled (orphans)` 分桶；`sites-available` 行尾有 enabled / disabled toggle，点击立即 `ln -sf` 或 `rm`，操作完自动 refresh。
- **结构化编辑**（右栏，默认模式）：
  - 整个文件以可折叠 `.ngx-card` 树呈现；顶层指令默认展开，深层默认折叠。
  - 高频指令给精细表单：
    - `listen` → 监听值 + ssl / http2 / default_server 多 flag 勾选
    - `server_name` → chip-list 多 host 增删
    - `root` / `proxy_pass` / `ssl_certificate` / `ssl_certificate_key` → 单值 path 输入（额外参数照原样保留）
    - `location` → modifier 下拉（`(prefix)` / `=` / `^~` / `~` / `~*`）+ path 输入
    - `upstream` → 名称输入（成员靠块内的 `server …;` 子卡片处理）
  - 其余指令 fallback 到一行 quote-aware 输入框；带 `_by_lua_block` 后缀的指令 body 渲染为只读 `<pre>`。
  - 改动通过本地 TS renderer 立即生成新文本到「dirty buffer」，按「保存」时把这个 buffer 发给 backend。
- **「原文」模式**：纯 textarea，等宽字体，专门给「Lua 块要改」「快速粘贴大段配置」等场景。Structured ↔ Raw 切换不会丢内容（Structured 改 → AST → render，Raw 改直接覆盖 dirty buffer）。
- **保存语义**（写操作的安全模型）：
  1. `cp -p <path> <path>.pier-bak.<ts>` 先做时间戳备份。
  2. 新内容 base64 编码 → `/tmp/pier-nginx-<ts>.conf` → `mv` 原子覆盖目标文件。
  3. 跑 `nginx -t`（针对**整个**配置树，因为 conf.d / sites-available 不能独立校验）。
  4. 校验失败：`mv <bak> <path>` 把原文件 mv 回去；面板保留备份路径方便排查。
  5. 校验通过：`systemctl reload nginx`（或 `nginx -s reload`）；备份保留在磁盘上等 ops 自己决定何时清理。
- **sudo**：非 root 时 cp / mv / nginx -t / reload / ln / rm 都加 `sudo -n` 前缀；sudo 需密码会得到清晰错误提示，不在 UI 里要密码。
- **模块视图**：面板底部 `<details>` 里展示 `nginx -V` 解析出的 `--with-*` 列表；额外模块（headers-more / geoip2 / nginx-extras）通过 Software 面板装，本面板不重复实现包管理逻辑。
- **不做（v1）**：
  - 配置语法补全 / 智能提示（IDE 风的 LSP）
  - SSL 证书自动签发（cert-manager / certbot 集成）
  - 日志面板联动（已有 Log 面板）
  - 在 UI 里编辑 lua / njs 嵌入脚本（保留为 opaque blob，要改请切原文模式）
  - 多文件批量改 / 全文搜索替换
  - 自动 reload（编辑完不自动跑校验，必须显式按「保存」或「校验」）

#### 5.12.2 Apache 子面板（9 项 feature catalog + 站点 toggle）

- **数据源 / 命令**（全部远端执行，sudo -n 透传非 root 账号）：
  - 探测：`command -v apache2 / httpd && <binary> -v` / `<binary> -M | head -c 4096` / `systemctl is-active <unit>`
  - 文件树：Debian 走 `/etc/apache2/{apache2.conf, ports.conf, conf-{available,enabled}, sites-available}`，RHEL 走 `/etc/httpd/{conf/httpd.conf, conf.d}`，运行时探测分支
  - 校验：`apachectl configtest` 优先，回落 `apache2ctl` / `httpd -t`
  - Reload：`systemctl reload apache2 || systemctl reload httpd || apachectl graceful`
  - 站点 toggle：`a2ensite` / `a2dissite` 优先，回落 `ln -sfn` / `rm -f` 在 sites-enabled 下手动管理
- **解析器**：`pier-core::services::apache::parse` / `render`，纯 Rust 行式解析（每行一个逻辑指令，`\<newline>` 拼接，`<Section args>...</Section>` 递归嵌套）。AST 与 nginx / caddy 同形（Directive / Comment + 可选 section body）。7 个单元测试覆盖 vhost / nested directory / IfModule / 引号 / 注释 / line continuation。
- **三种模式**（toolbar segmented，与 caddy 共享样式）：
  - **Features**（默认）：scope 选择器 → 功能卡片网格。scope 自动收集所有 `<VirtualHost>` + 顶层 `main`。`<IfModule>` / `<IfDefine>` 包裹的 vhost 会穿透显示。
  - **Raw**：textarea 编辑，与其他产品一致的 toolbar（New site / Validate / Reload / Save）。
  - Tree 模式当前**未实现**（解析器已就位，UI 待补）。
- **Feature catalog**（9 项，分 9 组）：
  | 组 | 功能 | 主要指令 |
  |---|---|---|
  | identity | ServerName + DocumentRoot | `ServerName` / `ServerAlias` / `DocumentRoot` |
  | tls | SSL / HTTPS | `SSLEngine` / `SSLCertificateFile` / `SSLCertificateKeyFile` / `SSLProtocol` |
  | proxy | Reverse proxy (mod_proxy) | `ProxyPass` / `ProxyPassReverse` / `ProxyPreserveHost` |
  | alias | Path alias (mod_alias) | `Alias` |
  | rewrite | Rewrite engine | `RewriteEngine on` + `RewriteRule …` |
  | headers | Security headers | `Header always set X-Frame-Options / X-Content-Type-Options / Strict-Transport-Security` |
  | auth | Basic authentication | 生成 `<Location />` 内嵌 `AuthType Basic` + `AuthUserFile` + `Require valid-user` |
  | directory | Directory access | `<Directory>` + `Require all granted/denied` + `AllowOverride All` |
  | logging | Access & error logs | `ErrorLog` / `CustomLog` |
- **+ New site 向导**：toolbar 上的 `+` 按钮弹出对话框，生成完整的 `<VirtualHost *:80>` / `<VirtualHost *:443>` 模板（ServerName + ServerAlias + DocumentRoot + `<Directory>` + 可选 SSL + ErrorLog/CustomLog），写入 `sites-available/<name>.conf`，可选自动 `a2ensite`。
- **保存语义**：与 nginx 同构（cp -p 备份 → base64 → 原子 mv → `apachectl configtest` → 失败 mv 回滚 → 通过则 `systemctl reload`）。

#### 5.12.3 Caddy 子面板（9 项 feature catalog + 树形视图）

- **数据源 / 命令**：
  - 探测：`command -v caddy && caddy version` / `caddy list-modules | head -c 4096` / `systemctl is-active caddy`
  - 文件树：`/etc/caddy/Caddyfile` + `/etc/caddy/conf.d/*`（如果主 Caddyfile 没写 `import conf.d/*`，conf.d 下的文件不会被加载——面板会在新建站点时提示这点）
  - 校验：`caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile`
  - Reload：`systemctl reload caddy` 或 `caddy reload --config <path> --adapter caddyfile`
- **解析器**：`pier-core::services::caddy::parse` / `render`，纯 Rust（约 660 行）。Caddyfile 是行式 + brace-嵌套语法，支持双引号 / 反引号 / 行续 / heredoc / `(snippet)` / 全局选项块 / matcher（`@name`）。5 个单元测试。
- **三种模式**：
  - **Features**（默认）：scope 选择器（global / site / snippet）→ 功能卡片。
  - **Tree**（只读）：把整个 AST 渲染为可折叠 `.ws-tree-card` 树，dirty buffer 实时反映在树里。
  - **Raw**：textarea。
- **Feature catalog**（9 项，分 8 组）：
  | 组 | 功能 | 主要指令 |
  |---|---|---|
  | tls | TLS | `tls <email>` / `tls <cert> <key>` / `tls internal`（三种模式 select） |
  | proxy | Reverse proxy | `reverse_proxy <upstream>` |
  | static | Static file server | `root * <path>` + `file_server [browse]` |
  | performance | Compression | `encode gzip zstd` |
  | headers | Security headers | `header { X-Frame-Options / X-Content-Type-Options / Strict-Transport-Security / Referrer-Policy }`（保留用户自加的其他 header 不删） |
  | auth | Basic auth | `basicauth { user hash }`（提示用 `caddy hash-password` 生成哈希） |
  | routing | Rewrite + Redirect | `rewrite from to`（服务端内部）/ `redir target [permanent|temporary|html]`（客户端 301/302） |
  | logging | Access log | `log { output file <path>; format default|json }` |
- **+ New site 向导**：生成 `<address> { reverse_proxy <upstream> } 或 { root * <path>; file_server } [+ encode gzip zstd] [+ log block]`，写入 `/etc/caddy/conf.d/<name>.caddyfile`。Caddy 没有 per-file 的 enable/disable，向导成功后会提示用户在主 Caddyfile 里加 `import conf.d/*` 才会被加载。
- **保存语义**：与 apache / nginx 同构（cp -p → 原子 mv → `caddy validate` → 失败回滚 → 通过则 `systemctl reload caddy`）。

#### 5.12.4 共性：探测 + 校验 + 重载

| 操作 | nginx | Apache | Caddy |
|---|---|---|---|
| 探测 | `nginx -v` | `apache2 -v` / `httpd -v` | `caddy version` |
| 模块列表 | `nginx -V` | `apachectl -M` | `caddy list-modules` |
| 配置根 | `/etc/nginx` | `/etc/apache2` 或 `/etc/httpd` | `/etc/caddy` |
| 校验 | `nginx -t` | `apachectl configtest` | `caddy validate` |
| 重载 | `systemctl reload nginx` ↻ `nginx -s reload` | `systemctl reload apache2/httpd` ↻ `apachectl graceful` | `systemctl reload caddy` ↻ `caddy reload` |
| 站点 toggle | `ln -sf` 在 sites-enabled | `a2ensite` / `a2dissite` ↻ `ln -sf` | n/a（用 `import` 行管理） |
| 解析器 | 纯 Rust，22 features | 纯 Rust，9 features，7 测试 | 纯 Rust，9 features，5 测试 |

---

## 6. TopBar / StatusBar / 对话框

### 6.1 TopBar

- 左：App 图标 / 名称 / 版本。
- 右：新建 tab、切换主题、设置、（macOS 用自定义 traffic lights 区域）。
- 不承载应用菜单（没有传统 macOS menubar）。

### 6.2 StatusBar

- 版本号、当前 tab 的 backend 摘要、运行时提示（bell pending 等）。

### 6.3 对话框

- **SettingsDialog**：
  - 主题（dark / light / system）
  - 终端主题（6 色板：Default Dark/Light、Solarized Dark、Dracula、Monokai、Nord）
  - 字体族（mono font 列表）、字号、光标样式（block/underline/bar）、光标闪烁、滚动回溯行数
  - Bell：可视 / 音频
  - 语言（en / zh）
- **NewConnectionDialog**：
  - 名称、host、port、user、认证方式（密码 / key file / agent）
  - 密码字段走 keyring；编辑已有连接时密码占位"留空则保留"，不回显明文
- **CommandPalette**（`⌘K` / `Ctrl+K`）：
  - 新建本地终端、新建 SSH、关闭 tab、设置、切换主题、切换到任一工具面板
  - 方向键 / 回车选择，Esc 关闭
- **PortForwardDialog**（从命令面板 / Help 菜单打开）：
  - 列出所有活动的 SSH local forward（tunnel_id / remote host:port / local port / 源 SSH 连接）。
  - 表单新增 local forward：选择 SSH 连接 + remote host + remote port + local port（0 = 自动）。
  - 逐条关闭（调 `ssh_tunnel_close`），或全部关闭。
  - **只支持 local forward（`ssh -L` 等价）**。Remote forward（`ssh -R`）需要 russh `tcpip_forward`，不在当前实现范围内；要用请先在终端里用 `ssh -R` 手动开。
  - 这个对话框是"可见现有 tunnel + 手动开新 tunnel"的入口；DB / Log 面板自动开的 tunnel 也会在这里显示，关掉会影响对应 panel。

---

## 7. 跨功能能力

### 7.1 快捷键

| 快捷键 | 动作 |
|---|---|
| `⌘K` / `Ctrl+K` | 命令面板 |
| `⌘T` / `Ctrl+T` | 新本地终端 |
| `⌘N` / `Ctrl+N` | 新 SSH 连接 |
| `⌘W` / `Ctrl+W` | 关闭当前 tab |
| `⌘,` / `Ctrl+,` | 设置 |
| `⌘⇧G` / `Ctrl+Shift+G` | 切到 Git 面板 |
| `F12` / `⌘⌥I` / `Ctrl+Shift+I` / `Ctrl+Shift+J` | Release 下屏蔽 DevTools |

全局 `contextmenu` 被禁用（除了终端视口和 input/textarea）。自定义右键菜单由各 panel 实现。

### 7.2 主题系统

- 单源 CSS 变量：`src/styles/tokens.css`，分 dark / light 两套。
- `data-theme="dark" | "light"` 挂在 `<html>` 根元素。
- `useThemeStore` 管 `mode: dark | light | system`、`resolvedDark: bool`；监听系统 `prefers-color-scheme`。
- 任何视图/面板/组件**只**引用 tokens，不写字面值（见 CLAUDE.md Rule 1）。

### 7.3 国际化

- 英文（en）/ 简体中文（zh），以 en key 为 fallback。
- `useI18n()` 提供 `t(key, vars?)`。
- 添加新字符串时：en 可以直接用 key 本身（自动 fallback），zh 必须补译。

### 7.4 凭证与安全

- SSH 密码 / key passphrase 一律走 `pier-core::credentials` → OS keyring。
- 密码不出现在：连接配置文件、日志、error message、Tauri invoke 的 debug trace。
- 已保存连接的密码在 UI 上不回显（只显示占位提示）。

### 7.5 日志文件

- 运行时日志写到 `pier-ui-gpui.log`（命名沿用旧项目，不是拼写错误）——未来重命名为 `pier-x.log`。不得记录密码、tunnel 凭证、SQL 参数里的敏感值。

---

## 8. pier-core 后端契约

前端对 `pier-core` 的假设（保持这些假设不变，面板才可信）：

### 8.1 Terminal

- `PierTerminal` 同步接口：`write_blocking(bytes)` / `snapshot()` / `resize(cols, rows)` / `close()`。
- Unix 用 forkpty + 非阻塞 I/O；Windows 用 ConPTY。
- VT100 状态机（vte crate）；未识别序列静默吞掉，不回显为乱码。
- scrollback 用环形缓冲，上限由上层设置。

### 8.2 SSH

- `russh` 异步，内部 tokio runtime；前端以 `*_blocking` 包装调用。
- 认证：密码 / 私钥文件（可带 passphrase）/ agent / keychain。
- 同会话支持多通道：一个 shell + N 个 exec + M 个 tunnel。
- `SshChannelPty` 把 SSH channel 适配成 `Pty` trait；上层 terminal 看到的是统一接口。
- Host key 校验：M3a 为 accept-all（**已知风险**，M3b 会引入 known_hosts）。前端不要假设现在是安全的。

### 8.3 服务客户端（service clients）

- 每个客户端（Git / MySQL / PG / SQLite / Redis / Docker）暴露**纯阻塞**的 pub API；底层是否 async 由客户端内部决定。
- 返回类型全部 `serde::Serialize`，能被 Tauri 直接透传给前端。
- `git` 客户端通过子进程执行 `git ...`，以 porcelain 格式解析；不直接 libgit2 except for graph layout（`git_graph.rs` 用 git2 做拓扑）。
- 数据库客户端默认**只读**语义由前端强制（`isReadOnlySql`）；后端执行什么 SQL 就返回什么结果，不做二次过滤。

### 8.4 Markdown

- `pulldown-cmark`，开启 tables / footnotes / strikethrough / task lists / heading attributes。
- 渲染后的 HTML 由前端 `dangerouslySetInnerHTML` 注入 `.markdown-preview` 容器（样式受 `shell.css` 里 `.markdown-preview .*` 规则约束）。

### 8.5 连接持久化

- `ConnectionStore`：YAML 文件（位置由 `pier-core::paths` 决定，跨平台 XDG）。
- `credentials`：keyring 键命名空间 `pier-x.*`。

---

## 9. 构建 / CI / 发布

- **开发**：在仓库根目录 `npm run tauri dev`
- **发布**：在仓库根目录 `npm run tauri build`
- **Cargo**：`cargo build -p pier-core` 构建纯后端。
- **版本更新**：`npm run bump <version>` 同步四处版本号（`package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`、`pier-core/Cargo.toml`）并创建 `v<version>` 标签。
- **CI**（`.github/workflows/ci.yml`）：
  - Tauri shell job：macOS + Windows 矩阵，构建 `--no-bundle`，扫描产物。
  - Rust core job：macOS + Windows + Linux，`fmt --check` + `clippy` + `build --release` + `test --release`。
- **Release**：
  - GitHub（`.github/workflows/release.yml`）：tag `v*.*.*` 触发，矩阵构建 Linux / Windows x64 / Windows ARM64 / macOS universal 四个平台 Tauri bundle，自动发布到 GitHub Releases。
  - Gitea（`.gitea/workflows/release.yml`）：同 tag 触发，`ubuntu-22.04` runner 构建 Linux `.deb` / `.rpm` / `.AppImage`，通过 Gitea API 上传到对应 Release。
- **Tauri 配置**（`src-tauri/tauri.conf.json`）：
  - `productName: "Pier-X"`；`identifier: "com.kkape.pierx"`。
  - 默认窗口 1600×980，最小 1200×760。
  - 标题栏 hidden overlay（自定义 traffic lights 区）。

---

## 10. 路线图锚点

**已完成**：terminal 引擎、SSH 会话 + service 探测、Git 深度面板、MySQL/PG/SQLite/Redis/Docker/SFTP/Markdown 面板、Windows + macOS CI、tag 触发发布。

**本期重点（Next up）**：
1. Terminal：scrollback UX、选区优化、稳定性。
2. Git：更完整的 remote 管理 / revert 流 / history graph UI。
3. Data panels：更强的结果表、更安全的写入流、保存的数据连接。
4. Service surfaces：PostgreSQL / Docker / SFTP / Server Monitor 打磨。
5. 工作区：键盘流、面板密度、设置清理。
6. Plugin host 边界（只做接口设计，不做实现）。

**长线但不在近期**：已知 host 验证 (M3b)、代码搜索 (M8)、commit signing、冲突的原生解决 UI、工作区状态恢复、RDP/VNC。

---

## 11. 术语表

| 词 | 含义 |
|---|---|
| **tab** | center 工作区的一个会话单元，携带 backend + rightTool + per-service 状态 |
| **backend** | tab 的运行载体：`local` / `ssh` / `sftp` / `markdown` |
| **rightTool** | 当前 tab 右侧 RightSidebar 显示哪个工具（`markdown` / `git` / `monitor` / …） |
| **ToolStrip** | 右侧窄竖条，切换 rightTool 的按钮组 |
| **browserPath** | 左侧 Sidebar 当前浏览到的本地路径；Git 面板就按这个路径去找仓库 |
| **selectedMarkdownPath** | 左侧 Sidebar 选中的 `.md` 文件路径；驱动 Markdown 面板渲染 |
| **tunnel** | SSH local port forward；MySQL / PG / Redis 远程连接用它转发数据库端口 |
| **service detection** | SSH 连上主机后探测对方装了哪些服务（MySQL / Redis / PG / Docker）及版本 |
| **known hosts** | SSH 首次连接的 host key 固定机制；Pier-X 目前**未**启用（M3b 待做） |

---

## 12. 修改本文档的规则

- 新增一个工具面板 / 右侧工具：**先改本文档第 5 节**，再写代码。
- 改变某个面板的默认安全策略（例如允许默认写 SQL）：**必须在 PR 里引用本文档修改理由**。
- 改动 keyboard shortcut、默认 rightTool、tab 颜色调色板：更新 §2.3 / §5 / §7.1 对应小节。
- 删除一个工具：一并删除本文档、ToolStrip、panel 文件、i18n 键，不留"隐藏入口"。
