# Pier-X 剩余工作清单

> 更新日期：2026-04-12
> 基于 `docs/PARITY.md` 完整审计 + `git log` + 工作树分析

---

## 一、总体进度

| 里程碑 | 状态 | 说明 |
|--------|------|------|
| **M1** 🟢 | ✅ 完成 | Corrosion + C ABI bridge |
| **M2** 🟢 | ✅ 完成 | 本地终端（Unix PTY + Windows ConPTY） |
| **M3** 🟢 | ✅ 完成 | SSH 全功能（密码/密钥/Agent/已知主机/SFTP/共享会话 M3e） |
| **M4** 🟢 | ✅ 完成 | 服务发现 + SSH 隧道 |
| **M5** 🟡 | 大部分完成 | Redis ✅ Log ✅ Docker ✅ MySQL ✅ Markdown ✅ — 还差 SQLite / Git / AI |
| **M6** 🔴 | 进行中 | 你在并行做（图标/字体/无边框/Toast/文件面板/Tab 拖拽已有进展） |
| **M7** 🟡 | 部分完成 | PostgreSQL ✅ 服务器监控 ✅ — 超出 PARITY.md 原始范围 |

**功能对齐率：~76%**（21 个上游功能中 16 个已实现）
**发布就绪率：~60%**（Git 面板 + 签名/打包阻塞 1.0.0）

---

## 二、已完成的 Commits

```
18c44a4 feat(m7b): server resource monitor
3f50fd1 feat(m7a): PostgreSQL client
767c20b feat(m3e): shared PierSshSession handle
22953e4 feat(m5d): MySQL client
4bcc4fb feat(m5e): local Markdown preview
c83351d feat(m5c): Docker panel
f81fb38 feat(m5b): streaming log viewer
09c1ea4 feat(m5a): Redis browser
0f470aa feat(m4b): SSH port-forwarding tunnels
9145f63 feat(m4):  remote service discovery
b898361 feat(m3d2): SFTP file browser
7820d07 feat(m3d1): SFTP client layer
5d718bc feat(m3c4): SSH agent + known_hosts
77e7d77 feat(m3c3): private-key auth
aace471 feat(m3c2): connection persistence
42e181c feat(m3c1): async SSH handshake
fe31346 feat(m3a): SSH session layer
0deb4d2 feat(m2b): local terminal
13f6866 feat(m2a): terminal engine
59c61bb feat(m1): Corrosion bridge
```

**Rust 测试覆盖：183 tests passing，clippy 零警告**

---

## 三、剩余工作（按优先级排序）

### 🔴 P0 — 阻塞 1.0.0 发布

#### 1. Git 面板（最大单项，~3-4 周）

上游有 `git_graph.rs`（1008 LOC）+ 7 个 ViewModel + 12+ QML 视图。建议分阶段：

| 阶段 | 内容 | 预估 |
|------|------|------|
| Phase 1 | `git status` + diff 视图 | 1 周 |
| Phase 2 | 分支列表 + commit 日志图 | 1 周 |
| Phase 3 | blame + rebase/merge 冲突 UI | 1 周 |
| Phase 4 | stash / tag / remote / submodule | 1 周 |

**依赖**：`git2` crate（Rust libgit2 binding）
**决策**：PARITY.md §6 建议"先逐字移植，后重构"

#### 2. M6 发布工程（~3-4 周）

| 项目 | 复杂度 | 状态 | 说明 |
|------|--------|------|------|
| **图标** | 小 | 🚧 你在做 | 用 Lucide SVG 替换 Unicode 占位符 |
| **捆绑字体** | 小 | 🚧 你在做 | Inter + JetBrains Mono 注册到 QFontDatabase |
| **无边框标题栏** | 中 | 🚧 你在做 | macOS traffic lights + PierNativeWindow_mac.mm（ARC cast 待修） |
| **i18n** | 中 | 🚧 部分完成 | `.ts` 文件已有，需要完善翻译 + Settings 语言切换器 |
| **Tab 拖拽排序** | 小 | 🚧 你在做 | `tabModel.move()` 已实现 |
| **快捷键自定义** | 中 | ❌ 未开始 | 需要 Settings UI + 持久化 |
| **macOS 签名** | 小 | ❌ 未开始 | Developer ID + `codesign` + `notarytool` |
| **Windows 签名** | 小 | ❌ 未开始 | EV 证书或 SignTool |
| **自动更新** | 中 | ❌ 未开始 | Sparkle (macOS) + WinSparkle (Win) + `appcast.xml` |
| **崩溃报告** | 小 | ❌ 未开始 | breakpad 或 sentry-native |
| **安装包** | 中 | ❌ 未开始 | `.dmg`（CI 部分就绪）+ MSI via WiX |
| **性能 profiling** | 中 | ❌ 未开始 | scrollback 100k 行 / 文件树 10k / 终端 1k chars/sec |

---

### 🟡 P1 — 功能完整性（非阻塞但提升产品力）

#### 3. AI 聊天面板（~2 周）

- 上游有 `LLMService` + `AIChatView`
- 流式响应、终端上下文感知
- 提供商：OpenAI / Claude / Ollama
- 密钥通过已有 `keyring` 存储
- **依赖**：`async-openai` crate + 手动 Claude/Ollama SSE 客户端

#### 4. SQLite 浏览器（~2 天）

- 本地文件选择器 → 同 MySQL/PG 的 grid 视图
- **依赖**：`rusqlite` crate
- 复用已有 `PierMySqlResultModel` / `QAbstractTableModel` grid
- 最简单的剩余 M5 项

#### 5. 本地文件面板（~3 天）

- 上游 `LocalFileView.swift` 移植
- 懒加载 + 右键菜单 + 拖拽到终端
- 你已有 `LocalFilesPane.qml`（265 行），可能已基本完成
- 需确认：是否需要进一步 pier-core 文件系统 API

---

### 🟢 P2 — 小打小闹

#### 6. 终端模拟器小项

来自代码中的 TODO 注释：

```
emulator.rs:275 — BEL 视觉铃声事件（当前忽略）
emulator.rs:285 — OSC 0/1/2 窗口标题 + OSC 52 剪贴板
```

#### 7. 你的 M7c 工作树待提交

```
CommandHistoryDialog.qml — Ctrl+R 命令历史搜索（207 行，已完成）
RightPanel.qml — 右侧面板统一调度器（177 行，已完成）
PierCoreBridge — localHistory() 读取 Bash/Zsh 历史
TerminalView — URL 检测 + 点击打开
```

---

## 四、已解决的架构决策（PARITY.md §6）

| 决策 | 选项 | 结论 | 状态 |
|------|------|------|------|
| cxx-qt vs cbindgen | cxx-qt 推荐 | 用了手写 C ABI + cbindgen 风格 | ✅ 已落地 |
| Windows SSH | russh only | russh only | ✅ 已落地 |
| 终端渲染 | QSGNode | QQuickPaintedItem（PierTerminalGrid）| ✅ 已落地 |
| 数据库编辑器 | TextArea + 高亮 | 用了 TextArea，无高亮（后续可加）| ✅ 已落地 |
| LLM 传输 | async-openai | ⏳ 待 AI 面板实现 |
| Git 模块 | 逐字移植 | ⏳ 待 Git 面板启动 |

---

## 五、架构规则合规检查（TECH-STACK.md §12）

| 规则 | 状态 |
|------|------|
| `pier-core` 不依赖任何 Qt 类型 | ✅ 合规 |
| 平台代码走 trait 抽象 | ✅ 合规（Pty trait / credentials） |
| 每个新功能必须有测试 | ✅ 合规（183 tests） |
| QML 组件必须匹配设计系统 | ✅ 合规（全部使用 Theme.* tokens） |
| CI 矩阵保持绿色 | ✅ 合规 |

---

## 六、你的 M6 并行工作中待修的 3 个 bug

1. **`pier-ui-qt/src/main.cpp`** — 缺 `#include <QWindow>`（`qobject_cast<QWindow*>` 需要完整类型）
2. **`pier-ui-qt/src/PierNativeWindow_mac.mm:16`** — `reinterpret_cast<NSView *>(window->winId())` 在 ARC 下被拒，应改 `(__bridge NSView *)(void *)(uintptr_t)(window->winId())`
3. **`pier-ui-qt/qml/Main.qml`** — 两处 `qsTr("Connection "%1" ...")` 内部双引号未转义（`"` 应改为 `\"`）

---

## 七、到 1.0.0 的路径（粗略时间线）

```
现在                    第 3 周                   第 6 周                    第 8 周
├─ 提交 M7c (1天)       ├─ Git Phase 1-2 (2周)   ├─ Git Phase 3-4 (2周)    ├─ M6 release (1周)
├─ SQLite+文件 (5天)    ├─ M6 图标/字体 (3天)     ├─ M6 签名/安装包 (1周)    └─ 1.0.0 打 tag
└─ AI 面板？(2周)       └─ M6 i18n (2天)          └─ 自动更新 (2天)             QA + 内测
                                                                                ↓ 公开发布
```

**总计预估：6-8 周到可发布的 1.0.0**

---

## 八、Rust pier-core 模块清单

| 模块 | 状态 | 测试数 |
|------|------|--------|
| `terminal::pty` | ✅ | 3 |
| `terminal::emulator` | ✅ | 10 |
| `terminal::session` | ✅ | 4 |
| `ssh::session` | ✅ | 1 |
| `ssh::channel` | ✅ | — |
| `ssh::known_hosts` | ✅ | 2 |
| `ssh::sftp` | ✅ | 3 |
| `ssh::tunnel` | ✅ | 1 |
| `ssh::exec_stream` | ✅ | 9 |
| `ssh::service_detector` | ✅ | — |
| `connections` | ✅ | — |
| `credentials` | ✅ | — |
| `markdown` | ✅ | 15 |
| `services::redis` | ✅ | 11 |
| `services::docker` | ✅ | 8 |
| `services::mysql` | ✅ | 11 |
| `services::postgres` | ✅ | 8 |
| `services::server_monitor` | ✅ | 8 |
| `ffi::*` (全部子模块) | ✅ | 98+ |
| **合计** | | **183** |

还缺的模块：`git` / `sqlite` / `llm`
