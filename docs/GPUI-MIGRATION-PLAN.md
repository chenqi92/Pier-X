# GPUI 迁移方案(草案 v1)— 垂直切片:TerminalPanel

> 状态:**待评审**。本文件是「用 GPUI 彻底替换 Tauri+React」决策下的第一步落地方案。
> 范围仅限第一个垂直切片(终端面板 + 最小 shell 外壳),不是全量迁移计划。
> 过目通过后再开工。

---

## 0. 目标与非目标

**本切片要证明三件事**(决定整条路走不走得通):

1. **好看** — GPUI + `gpui-component` + 移植的设计 token,能达到现在 Tauri 版的视觉水准(你当初放弃 GPUI 的唯一原因)。
2. **不卡** — 终端这种高频刷新负载,GPUI 直接画 `GridSnapshot` 比 WebView 画 DOM 网格更顺(回应你最初的卡顿诉求)。
3. **能接后端** — 验证 GPUI 直接调 `pier-core`、并重建会话注册表 + 事件通知这一薄层的可行性。

**本切片非目标**(明确推迟):

- 其余 24 个面板、17 个对话框、45+ 组件。
- smart mode 的完整 UI(autosuggest 幽灵文字、语法高亮 overlay、Tab 补全 popover、man-page popover)。引擎已在 pier-core,但 UI 推迟到切片验证通过后。
- 多 tab、命令面板、设置项全集、i18n 全量。切片只做单终端 + 最小 chrome。
- 删除任何 Tauri/React 代码。**双轨并行**:GPUI 版在独立 crate 里长出来,Tauri 版原样保留,直到 GPUI 达到可替换水准。

---

## 1. 关键架构事实(已核实)

| 事实 | 位置 | 对迁移的意义 |
|---|---|---|
| 终端引擎完全 UI 无关 | `pier-core/src/terminal/{session,emulator,pty}.rs` | GPUI 直接 `use pier_core::terminal::*`,零改动复用 |
| `GridSnapshot` + `write`/`resize`/`snapshot_view` | `pier-core/src/terminal/session.rs` | GPUI 的 paint loop 输入就是它,和现在前端拿的快照同构 |
| Unix forkpty / Windows ConPTY | `pier-core/src/terminal/pty.rs` | 跨平台 PTY 已就绪,无需重写 |
| smart mode 引擎(补全/历史/man/ssh 探测) | `pier-core/src/terminal/{completions,history,man,smart,ssh_watcher}.rs` | 逻辑在 Rust,GPUI 直接调;前端那 2827 行里很大一部分只是编排,不必照搬 |
| 会话注册表 + 事件转发(**唯一 Tauri 耦合**) | `src-tauri/src/lib.rs`(`terminals: Mutex<HashMap<String, ManagedTerminal>>`,notify→`terminal:event`) | **唯一需要为 GPUI 重建的薄层** |
| `pier-core` 不依赖 tauri | `pier-core/Cargo.toml`(已确认无 tauri) | UI-agnostic 红线完好,继续守住 |
| 终端**不用 xterm.js** | 前端自渲染 `TerminalSnapshot`→DOM 网格 | 渲染模型天然适配 GPUI 的 retained element 树 |

**结论**:终端切片 ≈ ①重建会话/事件薄层 + ②把 snapshot 画成 GPUI 元素 + ③键盘/resize 输入回灌。引擎不动。

---

## 2. 后端前置改造:抽出会话注册表

现在 `src-tauri/lib.rs` 同时承担「Tauri 命令外壳」和「终端会话注册表 + 事件通知」。GPUI 用不了 Tauri 的那部分。

**方案**:把与 UI 无关的「会话注册表 + notify 抽象」下沉到 `pier-core`(或新建 `pier-core/src/terminal/registry.rs`),让两个 UI 都复用:

```
pier-core/src/terminal/registry.rs   (新增)
  ├─ struct TerminalRegistry { sessions: Mutex<HashMap<SessionId, Session>> }
  ├─ fn create(...) -> SessionId
  ├─ fn write / resize / snapshot_view / close
  └─ notify 抽象:trait TerminalObserver { fn on_event(&self, id, kind); }
       — src-tauri 实现它 → emit "terminal:event"
       — pier-ui-gpui 实现它 → 推进 GPUI 的 channel / cx.notify
```

- `ManagedTerminal` 里**纯逻辑**部分(会话持有、PTY 读线程、scrollback limit、ssh watcher 挂载)移到 registry。
- `ManagedTerminal` 里**Tauri 专属**部分(`AppHandle::emit`)退化成 `TerminalObserver` 的一个实现,留在 src-tauri。
- 收益:① GPUI 直接用同一个 registry;② Tauri 版行为不变(只是 notify 走 trait);③ 继续遵守 `pier-core` 不依赖任何 UI crate 的红线(trait 是纯 Rust)。

> ⚠️ 这一步会改到现役的 Tauri 终端路径,需要回归测试现有终端功能不退化。是本切片**唯一**触碰生产代码的改动,务必小步 + 可回滚。

---

## 3. 新 crate 与 workspace 布局

```toml
# 根 Cargo.toml
members = [
  "pier-core",
  "src-tauri",
  "pier-ui-gpui",                       # 新增
  "tools/pier-x-completions-importer",
]
```

```
pier-ui-gpui/
├─ Cargo.toml
└─ src/
   ├─ main.rs            # gpui::App 启动,开窗
   ├─ theme.rs           # tokens.css → Rust Theme(见 §4)
   ├─ app.rs             # 根 view:最小 shell(标题栏 + 单面板槽位)
   ├─ shell/
   │   ├─ titlebar.rs    # 窗口控制(可后置)
   │   └─ statusbar.rs   # cols×rows、cwd、shell user(可后置)
   └─ terminal/
       ├─ view.rs        # TerminalView:持有 SessionId,订阅事件,paint
       ├─ grid.rs        # GridSnapshot → GPUI 元素(核心 paint loop)
       ├─ input.rs       # 键盘 → bytes → registry.write
       └─ color.rs       # 移植 resolveTerminalColor + TERMINAL_THEMES 调色板
```

> 命名沿用文档里曾出现的 `pier-ui-gpui`。注意:**这与 `CLAUDE.md` 当前「不要重新引入 pier-ui-gpui」的条款直接冲突**,见 §10,需先更新决策文档。

---

## 4. 主题桥:tokens.css → Rust Theme

GPUI 之前「太丑」的根因 = 没有设计系统。直接把现成 token 翻译成 Rust:

```rust
// theme.rs — 数值直接取自 src/styles/tokens.css(暗色 :root / 亮色 [data-theme=light])
pub struct Theme {
    // 背景
    pub bg: Hsla,         // --bg        #0e1116
    pub surface: Hsla,    // --surface   #12161d
    pub panel: Hsla,      // --panel     #1a202b
    // 文字
    pub ink: Hsla,        // --ink       #e5e9f0
    pub muted: Hsla,      // --muted     #747d8b
    // 线
    pub line: Hsla,       // --line      #242a36
    // 强调
    pub accent: Hsla,     // --accent    #4aa3ff
    // 状态
    pub pos: Hsla, pub neg: Hsla, pub warn: Hsla, pub info: Hsla,
    // 间距 / 圆角 / 字号(取自 --sp-*, --radius-*, --size-*)
    pub sp: [Pixels; N], pub radius_md: Pixels, ...
    // 字体
    pub sans: SharedString,  // "IBM Plex Sans"
    pub mono: SharedString,  // "IBM Plex Mono" / "JetBrains Mono"
}
```

要点:
- **暗 + 亮两套**,和 tokens.css 的 `:root` / `[data-theme=light]` 一一对应。
- **字体务必显式加载** IBM Plex Sans / Mono(GPUI `App::load_font` / asset),不要用系统默认 —— 这是「好看」的一半。
- 终端调色板单独走 `color.rs`:移植 `TERMINAL_THEMES` + `resolveTerminalColor`(`""` / `ansi:N` / `#rrggbb` 三种标签,16 色/256 色立方/灰阶的算法照搬)。
- 接 `gpui-component`(longbridge,shadcn 风)的 `ThemeColor`:把上面的语义色喂给它,组件(按钮/输入/表格)直接复用它的样式,避免手搓 chrome。

---

## 5. 终端 paint loop / 输入 / 事件(切片主体)

**渲染**(`grid.rs`):`GridSnapshot.lines[].segments[]` → 等宽 mono 文本行。每个 segment 一个带 fg/bg 的 styled text run。光标按 `cursorX/Y` + cursorStyle 画方块/竖线。这与现在前端 `terminal-screen` 的 DOM 网格同构,但直接走 GPU,无 reflow/重绘开销。

**输入**(`input.rs`):GPUI `KeyDownEvent` → 复用前端 `controlKeyMap` 的映射规则 → bytes → `registry.write(id, bytes)`。粘贴走 clipboard → write。

**resize**:GPUI 拿到 viewport 像素 + 字符 cell 尺寸 → 算 cols/rows → `registry.resize`。前端那套「拖拽中 pending、松手发一次 SIGWINCH」的去抖逻辑同样适用,在 GPUI 里用 drag 状态实现。

**事件通知**(替代 Tauri `listen`):registry 的 `TerminalObserver` 实现把 `on_event` 转成 GPUI 侧的信号 —— 用 `cx.spawn` + channel,或 entity 的 `cx.notify()`,触发 `TerminalView` 重新 `snapshot_view` 并 paint。后端已把输出 coalesce 到 ≤16ms,GPUI 侧每帧最多拉一次快照即可。

**生命周期**:view 创建时 `registry.create`,drop 时 `registry.close`。scrollback offset、bell(visual/audio)、activation 等细节按需补,不阻塞「能跑」。

---

## 6. 推迟项(切片不做,验证通过后再排)

- smart mode 三件套 UI(autosuggest / 语法高亮 / Tab popover)、man-page popover —— **引擎已在 pier-core**,只差 GPUI overlay。
- SSH 创建 / 保存连接 / 密码捕获 / 右侧栏联动。
- 多 tab、命令面板、设置全集、i18n。
- 其余面板。

---

## 7. 验收标准

1. **视觉**:GPUI 终端与现 Tauri 终端并排截图,配色/字体/间距/圆角一致或更好。主观「不丑」过关。
2. **性能**:`yes` / `docker logs -f` / 大量彩色输出 下,GPUI 版 CPU 占用与掉帧明显优于(至少不劣于)Tauri 版;输入延迟体感更低。
3. **功能**:本地 shell 创建、输入回显、resize 重排、scrollback、关闭无泄漏。
4. **回归**:§2 的 registry 抽取后,现役 Tauri 终端全部功能不退化。

任一不过关 → 在切片成本内止损,不铺后续面板。

---

## 8. 里程碑顺序

- **M0 基建**:建 `pier-ui-gpui` crate + 接 `gpui`/`gpui-component` + `theme.rs`(token 移植)+ 加载字体 → 跑出一个能开、配色正确的空窗口。
- **M1 后端薄层**:§2 抽出 `TerminalRegistry` + `TerminalObserver`,src-tauri 改用 trait,**回归现有终端**。
- **M2 paint**:`grid.rs` 把 `GridSnapshot` 画出来(先静态快照,再接事件刷新)。
- **M3 交互**:键盘输入 + resize + 关闭,本地 shell 端到端可用。
- **M4 评审**:截图 + 性能对比,出结论(继续 / 止损)。

---

## 9. 风险

| 风险 | 缓解 |
|---|---|
| GPUI pre-1.0,API 有破坏性变更 | 锁定具体版本;`gpui-component` 跟随其验证过的 gpui 版本 |
| §2 registry 抽取动到生产终端 | 小步提交、保留 Tauri 行为、回归测试 |
| 双轨期维护成本翻倍 | 切片限定单面板,尽快出 M4 结论;不通过则只亏一个面板 |
| 字体/字形渲染差异致「还是丑」 | M0 就验证字体加载,而非留到最后 |
| 全量迁移 ~83.5k 行 TS 的长尾 | 本方案只承诺切片;全量另立计划,按 paint loop 范式复制 |

---

## 10. 需先更新的决策文档(动工前)

「彻底替换 Tauri+React」推翻了仓库现有的既定决策,**动手前需更新**,否则违反 review gate 第 3 条:

- `CLAUDE.md`:删除/改写「不要重新引入 `pier-ui-gpui` / GPUI 已废弃」「IPC 是唯一桥、前端不得绕过 Tauri 调 pier-core」等条款 —— GPUI 版正是直接调 pier-core。需重写架构边界章节,描述「双轨过渡期」与终态。
- `docs/PRODUCT-SPEC.md`:若切片改变默认行为需同步(review gate 第 8 条)。本切片不改面板语义,暂不涉及。
- `AGENTS.md`:构建/评审规则补充 GPUI crate 的构建命令(`cargo run -p pier-ui-gpui`)。

> 建议:决策文档的改写**在 M4 评审通过、确认要全量替换之后**再正式落地;切片探索期可在 `CLAUDE.md` 顶部加一条「GPUI 迁移探索中,见 docs/GPUI-MIGRATION-PLAN.md」临时说明,避免推翻尚未验证的方向。
