<div align="center">
  <img src="public/pier-icon.png" alt="Pier-X" width="96" />
  <h1>Pier-X</h1>
  <p><strong>把终端 / Git / SSH / 数据库 / AI 助手 / 远程运维放进一个 IDE 风格工作台的桌面工具。</strong></p>
  <p>
    <a href="README.md">中文</a> ·
    <a href="README.en.md">English</a>
  </p>
</div>

---

Pier-X 是 [Pier](https://github.com/chenqi92/Pier)（仅 macOS）的跨平台继任者：同样的名字、同样的目标，给后端 / 运维工程师一台“不用切应用”的工作台。技术栈换成 **Rust 核心 + Tauri 2 + React 19 + TypeScript**，首发覆盖 **macOS** 与 **Windows**，长期保留 Linux。

> 完整产品规范见 [docs/PRODUCT-SPEC.md](docs/PRODUCT-SPEC.md)；视觉规范见 [.agents/skills/pier-design-system/SKILL.md](.agents/skills/pier-design-system/SKILL.md)；代码规则见 [CLAUDE.md](CLAUDE.md)。

## 界面截图

<table>
  <tr>
    <td width="50%" align="center"><img src="screenShots/ai.png" alt="AI 助手" width="100%"><br><sub><b>AI 助手</b></sub></td>
    <td width="50%" align="center"><img src="screenShots/git.png" alt="Git" width="100%"><br><sub><b>Git</b></sub></td>
  </tr>
  <tr>
    <td width="50%" align="center"><img src="screenShots/md.png" alt="Markdown 预览" width="100%"><br><sub><b>Markdown 预览</b></sub></td>
    <td width="50%" align="center"><img src="screenShots/监控.png" alt="服务器监控" width="100%"><br><sub><b>服务器监控</b></sub></td>
  </tr>
  <tr>
    <td width="50%" align="center"><img src="screenShots/docker.png" alt="Docker" width="100%"><br><sub><b>Docker</b></sub></td>
    <td width="50%" align="center"><img src="screenShots/sftp.png" alt="SFTP" width="100%"><br><sub><b>SFTP</b></sub></td>
  </tr>
  <tr>
    <td width="50%" align="center"><img src="screenShots/db.png" alt="数据库" width="100%"><br><sub><b>数据库（MySQL / PostgreSQL）</b></sub></td>
    <td width="50%" align="center"><img src="screenShots/redis.png" alt="Redis" width="100%"><br><sub><b>Redis</b></sub></td>
  </tr>
  <tr>
    <td width="50%" align="center"><img src="screenShots/web.png" alt="Web Server" width="100%"><br><sub><b>Web Server</b></sub></td>
    <td width="50%" align="center"><img src="screenShots/软件.png" alt="软件管理" width="100%"><br><sub><b>软件管理</b></sub></td>
  </tr>
  <tr>
    <td width="50%" align="center"><img src="screenShots/日志.png" alt="日志" width="100%"><br><sub><b>日志</b></sub></td>
    <td width="50%" align="center"><img src="screenShots/端口.png" alt="端口转发" width="100%"><br><sub><b>端口转发</b></sub></td>
  </tr>
</table>

## 功能一览

界面采用“**左侧 Sidebar + 中心 Tab 工作区 + 右侧工具面板**”的三栏 IDE 布局。每个 Tab 携带自己的右侧工具偏好，SSH Tab、数据库连接、SFTP 路径、AI 会话等状态都随 Tab 切换。

### 中心工作区

- **终端**：基于 xterm.js + `pier-core::terminal::PierTerminal`。
  - 三种后端：本地 PTY（forkpty / ConPTY）、SSH shell、已保存 SSH 连接（从系统 keyring 解析凭证）。
  - 支持 256 / RGB 色、SGR、可视 / 音频 bell、scrollback、复制选区 / 粘贴剪贴板、自定义右键菜单。
  - **Smart Mode** 可选开启：语法高亮、Tab 补全、历史 / autosuggest、man page 摘要、命令库导入；历史默认仅内存，可选择落盘并过滤常见敏感行。
- **Markdown**：选中左侧 `.md` 文件后自动渲染（pulldown-cmark，CommonMark + GFM）。
- **欢迎页**：无 Tab 时展示常用动作（新建本地终端 / 新建 SSH / 最近连接 / 设置 / 命令面板）。

### 左侧 Sidebar

- **Files**：以家目录为入口，面包屑 + Places 下拉；单击 Markdown 自动预览，双击目录在该目录打开本地终端。
- **Servers**：已保存 SSH 连接（YAML + 系统 keyring），支持搜索、编辑、删除、分组、健康探测；点击直接打开 SSH Tab。
- **Egress Profile**：连接可绑定 SOCKS5 / HTTP / SSH jump / WireGuard / external VPN 出站通道。代理和 jump host 只作用于绑定连接；系统级 VPN 明确提示路由影响。

### 右侧工具面板（per-Tab）

| 工具 | 适用 | 功能要点 |
|---|---|---|
| **AI** | 任意 | BYOK AI 助手，支持 Anthropic / OpenAI-compatible / Ollama；per-tab 对话、流式输出、上下文脱敏、历史可仅内存；`run_command` / `read_file` / `list_dir` / `write_file` / `monitor_snapshot` 走 L0-L3 风险分级与审批卡片 |
| **Markdown** | 任意 | 渲染左侧选中的 Markdown 文件 |
| **Git** | 任意 | 总览 / diff / 暂存 / 提交 / 推拉 / 分支 / 历史图 / blame / stash / tags / remotes / config / rebase / submodules / conflicts |
| **Server Monitor** | 本地 / SSH | uptime、load、CPU、内存、swap、磁盘、块设备拓扑、网络吞吐、Top 进程；本地走 `sysinfo`，远端走 SSH probe |
| **Firewall** | SSH | 自动探测 firewalld / ufw / nft / iptables；Listening / Rules / Mappings / Traffic；写操作注入终端等待用户审阅，不静默执行 |
| **SFTP** | SSH | 远程文件浏览、上传 / 下载（进度事件）、目录传输、chmod、复制 / 重命名 / 删除、书签、外部编辑、CodeMirror 内嵌编辑器（5 MB 上限、编码 / EOL 检测） |
| **Log** | SSH | File / System / Custom 日志源；journalctl、dmesg、nginx、docker logs 等预设；搜索、级别统计、速率 chip、时间范围回填、前端 drain 模型 |
| **Code Search** | SSH | 在终端 cwd 下优先用 `rg`，回退 `git grep`；支持正则、大小写、glob；命中点击后在 SFTP 编辑器定位 |
| **Docker** | 本地 / SSH | Containers / Images / Volumes / Networks / Projects；start / stop / restart / remove / inspect / pull / prune；Compose 项目按标签派生，不依赖 compose CLI |
| **MySQL / PostgreSQL** | SSH / 本地直连 | 自动 SSH tunnel、保存 DB 凭证、schema / table 浏览、SQL 多 Tab、历史 / 收藏、格式化、EXPLAIN / plan、结果表、TSV 导出、结构和变量编辑；默认只读，写入需解锁 + 确认 |
| **Redis** | SSH / 本地直连 | key tree、SCAN 分页、type / TTL / size、string / hash / list / set / zset / stream 详情、CLI 历史、编辑 / 删除 / rename、危险命令确认 |
| **SQLite** | SSH | 远端 sqlite3 探测、目录扫描 `.db` / `.sqlite`、表 / 索引 / trigger、PRAGMA、脚本执行、文件大小 chip；默认只读 |
| **Web Server** | SSH | 统一管理 nginx / Apache / Caddy：探测、文件树、Feature catalog、Tree / Raw 编辑、diff 预览、批量保存、validate / lint / reload、站点创建与 toggle、外部编辑 |
| **Software** | SSH | 主机软件清单、安装 / 更新 / 卸载 / 取消、版本选择、服务 start / stop / restart / reload、journal logs、镜像源切换、bundle、Compose 模板、K8s 导出、webhook、历史、`software-extras.json` 自定义条目 |

### 跨功能

- **命令面板**（`⌘K` / `Ctrl+K`）、AI 助手（`⌘⇧A` / `Ctrl+Shift+A`）、新终端（`⌘T` / `Ctrl+T`）、新 SSH（`⌘N` / `Ctrl+N`）、关闭 Tab（`⌘W` / `Ctrl+W`）、设置（`⌘,` / `Ctrl+,`）、Git 面板（`⌘⇧G` / `Ctrl+Shift+G`）。
- **主题与视觉令牌**：`dark` / `light` / `system`，所有视觉值来自 `src/styles/tokens.css`。
- **i18n**：英文 / 简体中文。
- **凭证与安全**：
  - SSH 密码、key passphrase、DB 密码、AI API key、Egress 代理凭证、sudo / elevation 密码均走系统 keyring。
  - SSH host key 使用 OpenSSH `known_hosts` 风格 TOFU：首次连接提示信任，变更时阻断并提示；设置页可查看 / 删除已固定 host key。
  - root 相关面板可使用已保存 sudo 密码执行 `sudo -S`，也可配置 SSH 连接打开后自动 `sudo -i`；明文不写配置和日志。
  - AI 执行通道由 `pier-core::services::ai::risk` 单点分级：L0 只读可自动执行，L1 需审批，L2 强确认且不可白名单，L3 红线永不代执行。
- **SSH Tunnel 管理**：`PortForwardDialog` 列出活动 local forward，可新增 / 关闭；DB / Log 面板自动开的 tunnel 也会显示。

## 架构

```
┌────────────────────────────────────────────────────┐
│        Tauri 2 + React 19 + TypeScript（shell）     │  src/
├────────────────────────────────────────────────────┤
│              Tauri 命令层（Rust）                    │  src-tauri/
├────────────────────────────────────────────────────┤
│              pier-core（Rust 核心）                  │  pier-core/
├────────────────────────────────────────────────────┤
│ PTY · SSH · SFTP · Git · DB · Redis · Docker · AI · │
│ Monitor · Firewall · Web Server · Software · …      │
└────────────────────────────────────────────────────┘
```

强约束（详见 [CLAUDE.md](CLAUDE.md)）：

- `pier-core` 不依赖任何 UI crate（`tauri` / `gpui` / `qt` 都不行）。
- 前端不绕过 Tauri 命令直连 `pier-core`。
- Tauri 命令是薄壳，业务逻辑都在 `pier-core`。
- 不引入 Qt / QML / CMake / Corrosion / `pier-ui-gpui` / 第二套 UI runtime。

## 构建与运行

### 环境

- Node.js 24+、npm 11+
- Rust 1.88+
- Windows 需 WebView2 运行时

### 命令

```bash
npm install                 # 第一次安装前端依赖
npm run tauri dev           # 开发：vite + tauri dev
npm run tauri build         # 发布构建
npm run build:debug         # 带调试符号的构建
cargo build -p pier-core    # 仅构建 Rust 核心
```

## 安装

当前正式安装方式是从 [GitHub Releases](https://github.com/chenqi92/Pier-X/releases) 下载对应平台安装包。每次 GitHub Release 会同时附带 Homebrew / WinGet 元数据：

- Homebrew cask：`pier-x-homebrew-cask-v<version>.rb`
- Homebrew formula：`pier-x-homebrew-formula-v<version>.rb`
- WinGet manifests：`pier-x-winget-manifests-v<version>.tar.gz`
- SHA256：`pier-x-release-sha256-v<version>.txt`

Homebrew tap 发布后，macOS 可用：

```bash
brew install --cask chenqi92/tap/pier-x
```

Linuxbrew 可用：

```bash
brew install chenqi92/tap/pier-x
```

WinGet manifest 被社区源接受后可用：

```powershell
winget install Chenqi92.PierX
```

维护流程见 [docs/PACKAGE-MANAGERS.md](docs/PACKAGE-MANAGERS.md)。

## 发布

版本号同步走脚本：

```bash
npm run bump <version>      # 显式版本
npm run bump patch          # patch / minor / major
git push
```

推送包含 `package.json` 版本变化的 `main` 分支提交后：

- **GitHub**（`.github/workflows/release.yml`）：构建 Linux / Windows x64 / Windows ARM64 / macOS universal Tauri bundle，发布到 GitHub Releases，并生成 Homebrew cask / WinGet manifests / SHA256SUMS。
- **Gitea**（`.gitea/workflows/release.yml`）：在 `ubuntu-22.04` runner 上构建 Linux `.deb` / `.rpm` / `.AppImage`，通过 Gitea API 上传到对应 Release。

CI（`.github/workflows/ci.yml`）：Tauri shell 在 macOS + Windows 上构建；Rust 核心在 macOS + Windows + Linux 上 `fmt --check` + `clippy` + `build` + `test`。

## 项目结构

```
Pier-X/
├── Cargo.toml               # Cargo workspace（成员：pier-core、src-tauri）
├── package.json             # 前端入口（npm run tauri …）
├── src/                     # React 前端（active desktop shell）
│   ├── shell/               # TopBar / Sidebar / TabBar / StatusBar / 对话框
│   ├── panels/              # 15 个右侧工具面板 + 终端 / WebServer 子面板
│   ├── components/          # 可复用 UI 原子
│   ├── stores/              # zustand 状态
│   ├── lib/                 # Tauri 命令包装、AI vendors、纯函数工具
│   ├── i18n/                # en / zh 资源
│   └── styles/              # tokens.css（视觉单源）+ 各域样式
├── src-tauri/               # Tauri 运行时 + Rust 命令桥
├── pier-core/               # Rust 核心（terminal / ssh / services / ai / …）
├── docs/
│   ├── PRODUCT-SPEC.md      # 产品规范（权威源）
│   ├── PACKAGE-MANAGERS.md  # Homebrew / WinGet 发布元数据流程
│   ├── BACKEND-GAPS.md      # 设计 → 实现差距追踪
│   └── SOFTWARE-EXTRAS.md   # Software 面板自定义条目格式
├── .agents/skills/          # 设计系统 SKILL 与仓库自动化
├── scripts/                 # 版本与发布辅助脚本
└── .github/ · .gitea/       # CI / Release workflows
```

## 文档索引

| 文档 | 作用 |
|---|---|
| [docs/PRODUCT-SPEC.md](docs/PRODUCT-SPEC.md) | 产品规范：“Pier-X 是什么、有什么面板、默认行为、不做什么”的权威来源 |
| [docs/PACKAGE-MANAGERS.md](docs/PACKAGE-MANAGERS.md) | Homebrew tap 与 WinGet manifest 的生成、发布、提交流程 |
| [docs/BACKEND-GAPS.md](docs/BACKEND-GAPS.md) | 前端设计 → 后端命令的差距清单 |
| [docs/SOFTWARE-EXTRAS.md](docs/SOFTWARE-EXTRAS.md) | Software 面板自定义软件 / bundle 条目的 JSON 格式 |
| [.agents/skills/pier-design-system/SKILL.md](.agents/skills/pier-design-system/SKILL.md) | 视觉令牌（颜色 / 排版 / 间距 / 圆角 / 阴影）唯一来源 |
| [CLAUDE.md](CLAUDE.md) | 给 AI / 协作者的代码规则与架构边界 |
| [pier-core/README.md](pier-core/README.md) | Rust 核心 crate 的对外契约 |

## License

MIT © 2026 [kkape.com](https://kkape.com)
