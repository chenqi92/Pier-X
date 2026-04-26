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

ToolStrip 顺序（工具栏第一位就是默认工具）：

| # | 工具 | 图标 | 作用域 | 远程必需？ |
|---|---|---|---|---|
| 1 | **Markdown** | FileText | 预览当前选中的本地 .md（来自左侧 Sidebar） | — |
| 2 | **Git** | GitBranch | 对当前浏览路径（`browserPath`）做 Git 操作 | — |
| 3 | **Server Monitor** | ActivitySquare | 远程主机状态快照 | 需 SSH tab |
| 4 | **Docker** | Container | 本地或远程 Docker 管理 | 支持两种模式 |
| 5 | **MySQL** | Database | 通过 SSH tunnel 到远程 MySQL，或本地 | 需 tab |
| 6 | **PostgreSQL** | Database | 同上 | 需 tab |
| 7 | **Redis** | Zap | 同上 | 需 tab |
| 8 | **Log** | ScrollText | 流式查看远程命令输出 | 需 SSH tab |
| 9 | **SFTP** | FolderTree | 远程文件浏览/上传/下载 | **仅** SSH tab |
| 10 | **Firewall** | Shield | 防火墙规则 / 监听端口 / 接口流量 / 端口映射 | 需 SSH tab |
| 11 | **SQLite** | HardDrive | 打开本地 `.db` 文件 | — |

ToolStrip 第 1 项和第 2 项之间有 divider，分隔"文档/工程"与"服务"。

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

### 5.5 MySQL / PostgreSQL 面板

- **连接建立**：
  - 如 SSH tab：自动开 SSH tunnel（`ssh_tunnel_open`）到远程 3306 / 5432，记录 `mysqlTunnelId` / `pgTunnelId` / `mysqlTunnelPort` / `pgTunnelPort` 到 tab state，连接走 `127.0.0.1:<localPort>`。
  - 本地直连也支持（填本地 host / port）。
- **浏览**：database / schema / table 三级选择器；表列 metadata 展示。
- **数据预览**：`SELECT * FROM <table> LIMIT N` 结果表（`PreviewTable`）。
- **查询编辑器**：原生 SQL 输入 + 执行（`mysql_execute` / `postgres_execute`）。
- **结果集**：`QueryResultPanel` 显示列、行、耗时、截断标记、影响行数、last insert id；支持导出为 TSV（`queryResultToTsv`）粘贴到表格软件。
- **安全模式（关键）**：
  - 默认只允许以 `SELECT / SHOW / DESCRIBE / EXPLAIN / PRAGMA / USE / SET / BEGIN / COMMIT / ROLLBACK` 等只读关键字开头的语句（`isReadOnlySql`）。
  - 写操作需要**显式解锁写入**（UI 开关），并在执行前再次确认。绝不能"智能识别无害 DELETE"。
  - 这个约束未来不能被放宽。

### 5.6 SQLite 面板

- 本地文件；通过"选择 .db 文件"输入路径。
- 表列表 / 列 metadata / 预览 / 查询同 MySQL 逻辑。
- 同样的只读默认 + 显式解锁规则。

### 5.7 Redis 面板

- 连接：同样支持 SSH tunnel。
- **扫描**：pattern (默认 `*`) + limit；超限截断提示。
- **key 详情**：type（string/list/hash/set/zset/stream）、TTL、编码、首若干成员预览。
- **命令编辑器**：空格分隔的 Redis 命令（例如 `SET foo bar`），返回摘要 + 多行输出 + 耗时。
- **危险动作**：`FLUSHDB` / `FLUSHALL` / `KEYS *`（大库阻塞）必须给醒目警告；UI 不禁用但要求二次确认。

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
