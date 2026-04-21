---
name: pier-design-system
description: Pier-X 视觉与交互的唯一设计标准。任何 Pier-X UI 工作 — 颜色、字体、间距、组件、对话框、面板、主题切换、动画 — 都必须遵守这套规范。当你为 Pier-X 编写 React/TSX、写 CSS、添加一个 panel/dialog/atom，或决定任何视觉细节时，应用这套 tokens。目标：在 Tauri 2 + React 19 + TypeScript 上做出 IntelliJ 级别的工程化深色 / 浅色 IDE 体验，跨 macOS 与 Windows。
type: reference
source-of-truth: pier-x-copy/index.html, pier-x-copy/Page.html, pier-x-copy/src/*.jsx
stack: Tauri 2 + React 19 + TypeScript + Vite 7
tokens-file: src/styles/tokens.css
atoms-file: src/styles/atoms.css
prototype-style: src/styles/pier-x.css
---

# Pier-X Design System

> **Engineered darkness, instrument precision.**
> 工程化的暗色，仪器级的精度。JetBrains 级 IDE 体验，专为跨平台终端管理工具设计。
>
> 综合自 Linear（luminance stacking）+ Warp（终端克制）+ Raycast（macOS 原生深度）+ JetBrains Darcula（熟悉的 IDE 心智）。
>
> **权威视觉参考**：`pier-x-copy/index.html` + `pier-x-copy/Page.html` + `pier-x-copy/src/*.jsx`。
> **React 实现**：仓库根目录 `src/`，tokens 在 `src/styles/tokens.css`，共享类在 `src/styles/atoms.css` / `src/styles/pier-x.css`。

---

## 1. 五条不可妥协的原则

| # | 原则 | 含义 |
|---|---|---|
| 1 | **Darkness is the medium, not a theme** | 深色是原生媒介，浅色是镜像。层级通过**亮度叠层**（`--bg → --surface → --surface-2 → --panel → --panel-2 → --elev`）传达，而非颜色变化 |
| 2 | **Single chromatic accent** | 全系统**只有一个**强调色：默认 `#4AA3FF`。用户可切 blue / green / amber / violet / coral，但**同一时刻只有一个**。状态色（pos/warn/neg/info）仅用于状态指示，**禁止装饰性用色** |
| 3 | **Semi-transparent or soft borders, never solid black / white** | 深色下用三阶灰 `--line / --line-2 / --line-3`，浅色下用暖灰 `--line / --line-2 / --line-3`。**禁止高对比黑线或白线** |
| 4 | **IBM Plex for everything** | Sans 用于 UI、Mono 用于代码 / 终端 / 路径 / IP / 端口 / 时间戳 / 数据单元格。Serif italic 仅 Welcome 页 hero。**500 是签名权重**，600 用于小号 mono uppercase 标签 |
| 5 | **Density over spectacle** | IDE 级工具，不是营销页。**compact 12px / comfortable 13px** 基础文字、4px 间距栅格、34px 面板头、26px 行高。不要巨型标题、不要装饰渐变 |

> **Review 反问**：这一改动是否违反了上面五条之一？如果是，**不要合并**。

---

## 2. 颜色 Tokens

所有颜色必须通过 CSS 自定义属性引用 `var(--*)`。`shell/`、`panels/`、`components/` 以及 `styles/` 下除 `tokens.css` 以外的任何文件中都**禁止**出现 hex / rgb / rgba / hsl 字面量。

### 2.1 深色主题（默认，`:root` / `[data-theme="dark"]`）

```
背景层（luminance stacking — 越高层 = 越亮）
─────────────────────────────────────────────
--bg               #0E1116   主窗口最深背景（canvas）
--surface          #12161D   停靠面板、侧边栏底色
--surface-2        #171C25   tab bar、panel header、状态栏
--panel            #1A202B   卡片、对话框、抬升表面
--panel-2          #222937   hover 态、二级表面
--elev             #252D3D   popover、菜单、tooltip
--bg-hover         rgba(255,255,255,0.05)   hover 叠加
--bg-active        rgba(255,255,255,0.08)   按下 / 活跃

文本
─────────────────────────────────────────────
--ink              #E5E9F0   主要文字（**不是**纯白 #FFF）
--ink-2            #B9C1CC   次要文字、描述、标签
--muted            #747D8B   弱化文字、占位、metadata
--dim              #4E5663   禁用、分隔符
--accent-ink       #0A1420   accent 按钮上的文字

边框（三阶灰）
─────────────────────────────────────────────
--line             #242A36   默认（输入框、卡片、面板）
--line-2           #2E3542   常规（突出的边界）
--line-3           #3A4254   强（重要分隔、按钮边框）

强调色
─────────────────────────────────────────────
--accent           #4AA3FF   默认 · IntelliJ Remix 蓝
--accent-hover     #6EB6FF   hover
--accent-dim       #1E3A5C   accent 背景填充（muted）
--accent-subtle    rgba(74,163,255,0.08)   极淡 accent 背景

状态色（两主题共享；仅用于状态指示）
─────────────────────────────────────────────
--pos              #3DD68C   运行中、成功
--pos-dim          #17402C   运行中背景色
--warn             #FFB547   警告
--warn-dim         #3E2C14   警告背景色
--neg              #FF5A5F   错误、拒绝
--neg-dim          #3E1C1F   错误背景色
--info             #7AA2F7   信息（辅助蓝，与 accent 区分）

Git diff / 语法高亮（语义复用状态色）
─────────────────────────────────────────────
--add              #3DD68C   等价 --pos
--del              #FF5A5F   等价 --neg
--mod              #FFB547   等价 --warn
```

### 2.2 浅色主题（`[data-theme="light"]`）— 暖象牙，镜像深色结构

```
背景层（避免纯白刺眼）
─────────────────────────────────────────────
--bg               #F5F3EE
--surface          #FBFAF5
--surface-2        #F3F1EA
--panel            #FFFFFF
--panel-2          #F9F7F1
--elev             #FFFFFF   靠阴影区分层级

文本
─────────────────────────────────────────────
--ink              #14171D   不是纯黑
--ink-2            #384050
--muted            #6E7585
--dim              #9AA0AD
--accent-ink       #FFFFFF

边框
─────────────────────────────────────────────
--line             #E4E0D4
--line-2           #D6D2C4
--line-3           #C6C1B0

强调色 / 状态色：与深色主题同值；仅 *-dim 改为高亮版
--accent-dim       #D6E6FA
--pos-dim          #CDEEDB
--warn-dim         #F7E4BF
--neg-dim          #F9D3D5
```

### 2.3 Accent 切换（`[data-accent="..."]`）

```
blue   (default)  --accent: #4AA3FF   --accent-dim dark/light: #1E3A5C / #D6E6FA
green             --accent: #3DD68C   --accent-dim dark/light: #17402C / #CDEEDB
amber             --accent: #FFB547   --accent-dim dark/light: #3E2C14 / #F7E4BF
violet            --accent: #B48CFF   --accent-dim dark/light: #2E2142 / #E4D8FA
coral             --accent: #FF7A59   --accent-dim dark/light: #3E1E14 / #F7DCCF
```

用户在 Settings → Appearance 切换后，`document.documentElement.dataset.accent` 变化，`tokens.css` 中 `[data-accent="..."]` 覆写生效。

### 2.4 服务品牌色（toolstrip 图标 tint，sidebar service chip）

```
--svc-docker    #4AA3FF
--svc-mysql     #F29D49
--svc-postgres  #8FB3FF
--svc-sqlite    #8AA0B8
--svc-redis     #E5484D
--svc-monitor   #3DD68C
--svc-log       #B48CFF
--svc-sftp      #8AA0B8
--svc-markdown  #7AA2F7
```

用于 sidebar 服务 chip（`.srv-svc`）、toolstrip 图标 tint（`--ts-tint`）。这是**唯一允许**按品牌语义用色的场景。

### 2.5 终端 ANSI 16 色（两主题通用）

终端区域使用专门 ANSI 调色板，与 UI chrome 解耦。

```
        Normal      Bright
black   #1C1E22     #5A5E66
red     #FF5A5F     #FF8593
green   #3DD68C     #7FCF85
yellow  #FFB547     #FFC15C
blue    #4AA3FF     #7CB9FF
magenta #C49EFF     #D894ED
cyan    #56E0C8     #7FC8D1
white   #B9C1CC     #E5E9F0
```

**规则**：UI chrome 永远不直接引用 ANSI 调色板；终端 xterm 实例通过 `WebglAddon` 的 theme 属性注入，UI tokens 保持不变。

---

## 3. 字体 Tokens

### 3.1 家族

```css
--sans:  "IBM Plex Sans", system-ui, -apple-system, "Segoe UI", "SF Pro Text", sans-serif;
--mono:  "IBM Plex Mono", "JetBrains Mono", "SF Mono", Consolas, ui-monospace, monospace;
--serif: "IBM Plex Serif", Georgia, "Times New Roman", serif;
```

- **家族固定**：禁止引入 Inter / Roboto / Nunito / 其他 sans；禁止用 JetBrains Mono 覆盖 Plex Mono（只允许在 fallback 链里）。
- **字体文件**：`src/styles/fonts.css` 引用本地 IBM Plex；离线时走系统 fallback。

### 3.2 字号阶梯

IDE 工具的基础文字是 **12px（compact） / 13px（comfortable）**，不是营销页的 16px。

| Role | Font | Size | Weight | Line-height | 用途 |
|---|---|---|---|---|---|
| Welcome hero | Serif italic | 44px | 400 | 1.10 | Welcome 页唯一 serif 使用点 |
| H1 (dialog) | Sans | 20–24px | 600 | 1.30 | 设置页 / 大型对话框标题 |
| H2 | Sans | 16–18px | 600 | 1.35 | 分组标题 |
| H3 | Sans | 14px | 600 | 1.40 | 卡片 / 分区 |
| Body Large | Sans | 14px | 400 | 1.50 | 主要阅读文本、dialog body |
| **Body** | **Sans** | **12px c / 13px cf** | **400** | **1.45** | **默认 UI 文字（最常用）** |
| UI Label | Sans | 12px | 500 | 1.0 | 按钮、tab、工具栏 |
| Caption | Sans | 11px | 500 | 1.40 | 辅助标签、列头 |
| Small | Sans | 11px | 400 | 1.40 | 提示 |
| Metadata | Mono | 10.5px | 400 | 1.40 | 状态栏、端口、IP、时间戳 |
| Mono Code | Mono | 12.5px | 400 | 1.45 | 终端、代码、SQL |
| Mono Small | Mono | 11.5px | 400 | 1.40 | 内联代码、路径、breadcrumb |
| **Panel Title** | **Mono** | **11.5px** | **600** | **1.0** | **PanelHeader（UPPERCASE, tracking 0.06em）** |
| Section Header | Mono | 10.5px | 600 | 1.0 | 面板内小分区（UPPERCASE, tracking 0.08em） |

**CSS 变量**（在 tokens.css 中，随 density 切换）：

```
--ui-fs        12px / 13px
--ui-fs-sm     11px / 12px
--ui-fs-lg     13px / 14px
--size-micro   10px
--size-small   11px
--size-body    12–13px
--size-h3      14px
--size-h2      16–18px
--size-h1      20–24px
--size-display 44px
```

### 3.3 字重 & 排版规则

- **500 是签名权重**（IBM Plex 的 medium）。UI 标签、按钮、菜单项默认 500；body 用 400；强调 / panel title 用 600；700 仅在极少数 dialog H1。
- **PanelHeader 与 SectionHeader 一律 Mono UPPERCASE**（`text-transform: uppercase; letter-spacing: 0.06–0.08em`）。这是 Pier-X 的"工程图"质感标志。
- **必须用 Mono 的内容**：代码、路径、IP、端口、命令、时间戳、文件大小、列头、数据库单元格、hash、git 引用。任何"机器可读"或"等宽对齐"的内容都是 Mono 范畴。
- **Serif italic 仅用于 Welcome 页 hero**。其他位置禁止混入 serif。
- **禁止第四种字体家族**。

---

## 4. 间距 Tokens（4px 栅格）

IDE 是高密度界面，**4px 栅格，不是 8px**。JetBrains 全套 IDE 都用 4px 增量。

```
--sp-0      0
--sp-0-5    2px    图标内部微调
--sp-1      4px    最小 gap
--sp-1-5    6px    inline 元素间
--sp-2      8px    组件内 padding 标准
--sp-3      12px   组件之间 gap
--sp-4      16px   区块内边距
--sp-5      20px   次级区块间
--sp-6      24px   主要区块间
--sp-8      32px   大区块
--sp-10     40px
--sp-12     48px   hero 间距
```

### 4.1 Density 切换（`[data-density="..."]`）

`compact`（默认）与 `comfortable` 切换影响 chrome 尺寸与 UI 字号：

| 变量 | compact | comfortable |
|---|---|---|
| `--row-h` | 26px | 32px |
| `--row-pad` | 8px | 12px |
| `--titlebar-h` | 36px | 40px |
| `--tabbar-h` | 34px | 38px |
| `--statusbar-h` | 24px | 26px |
| `--panel-header-h` | 34px | 38px |
| `--sidebar-w` | 244px | 260px |
| `--rightpanel-w` | 360px | 400px |
| `--toolstrip-w` | 42px | 46px |
| `--ui-fs` | 12px | 13px |

### 4.2 常用组合

- 按钮 padding：`var(--sp-2) var(--sp-3)`（8 × 12px）
- 输入框 padding：`var(--sp-1-5) var(--sp-2)`（6 × 8px）
- 卡片 padding：`var(--sp-4)`（16px）
- 对话框 body padding：`var(--sp-4)`
- 工具栏 gap：`var(--sp-1)`（按钮间）/ `var(--sp-2)`（分组间）
- 面板 gap：`var(--sp-3)`（内部 stack）/ `var(--sp-2)`（list 行间）
- **行与行之间永远有 gap 或 padding**，不允许 `gap: 0`（最小 `--sp-1`）

---

## 5. 圆角 Tokens

```
--radius-none   0
--radius-xs     2px     内联 badge、status dot、tab 颜色标
--radius-sm     4px     按钮、输入框、sidebar tab（默认）
--radius-md     6px     tab 顶圆角、卡片、popover、菜单
--radius-lg     8px     对话框、大卡片
--radius-xl     12px    大型 panel、overlay
--radius-pill   999px   状态药丸（.srv-svc / .db-badge / .mon-pill）
--radius-circle 50%     图标按钮、头像、status dot
```

**默认 `--radius-sm` (4px)**。IDE 风格是更克制的 4–6px，不是 macOS 那种 12–16px 大圆角。

---

## 6. 阴影 / 高度 / 遮罩 Tokens

### 6.1 深度模型

| 层级 | 实现 | 用途 |
|---|---|---|
| L0 Flat | 无阴影 + `--bg` | 主窗口背景 |
| L1 Panel | `--surface` / `--surface-2` | 停靠面板、侧边栏、状态栏（仅靠背景色区分） |
| L2 Surface | `--panel` + `--line` border | 卡片、面板内子容器 |
| L3 Elevated | `--panel` + `--line-2` border + `--shadow-app` | app 根容器、抬升的面板 |
| L4 Popover | `--elev` + `--shadow-popover` | 下拉菜单、tooltip、command palette |
| L5 Modal | `--elev` + `--shadow-dialog` + `--overlay-scrim` | 对话框、diff 弹窗 |

### 6.2 阴影定义（深色主题）

```
--shadow-app
  0 0 0 0.5px rgba(255,255,255,0.06)            外环高光
  0 40px 80px -20px rgba(0,0,0,0.6)             主投影
  0 18px 32px -12px rgba(0,0,0,0.5)             次投影

--shadow-popover
  0 0 0 1px rgba(0,0,0,0.40)                    外环
  0 8px 24px rgba(0,0,0,0.32)                   主投影
  0 2px 8px rgba(0,0,0,0.24)                    中投影
  inset 0 1px 0 rgba(255,255,255,0.05)          顶部高光（关键！）

--shadow-dialog
  0 0 0 1px rgba(0,0,0,0.50)
  0 24px 64px rgba(0,0,0,0.48)
  0 8px 24px rgba(0,0,0,0.32)
  inset 0 1px 0 rgba(255,255,255,0.06)
```

**核心规则**：深色主题的阴影必须包含 `inset 0 1px 0 rgba(255,255,255,0.05)` 顶部高光。这是 macOS 原生质感的来源 —— 让 popover 看起来像玻璃面板。

### 6.3 阴影定义（浅色主题）

```
--shadow-app
  0 1px 2px rgba(15,20,30,0.06)
  0 8px 24px rgba(15,20,30,0.10)

--shadow-popover
  0 0 0 1px rgba(15,20,30,0.08)
  0 8px 24px rgba(15,20,30,0.12)
  0 2px 8px rgba(15,20,30,0.08)

--shadow-dialog
  0 0 0 1px rgba(15,20,30,0.10)
  0 24px 64px rgba(15,20,30,0.20)
  0 8px 24px rgba(15,20,30,0.12)
```

### 6.4 遮罩

```css
--overlay-scrim      rgba(5, 7, 13, 0.55)       /* dark */
--overlay-scrim      rgba(230, 226, 215, 0.55)  /* light */
--stage-gradient     linear-gradient(180deg, var(--surface-2) 0%, var(--surface) 60%)
```

`backdrop-filter: blur(2px)` 应用于所有 dialog overlay（`.cmdp-overlay`、`.dlg-overlay`）。

### 6.5 焦点环

```
.srv-dot.on {
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--pos) 28%, transparent);
}
*:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 1px;
}
```

禁止 `outline: none` 不提供替代焦点环。

---

## 7. 动画 Tokens

```
--dur-instant    0ms
--dur-fast       120ms   hover、focus、颜色变化（默认）
--dur-normal     200ms   主题切换、状态转换
--dur-slow       320ms   面板滑入、模态进入
--dur-slower     480ms   大型布局变化

--ease-standard    cubic-bezier(0.4, 0.0, 0.2, 1)   默认
--ease-decelerate  cubic-bezier(0.0, 0.0, 0.2, 1)   入场
--ease-accelerate  cubic-bezier(0.4, 0.0, 1.0, 1)   出场
--ease-sharp       cubic-bezier(0.4, 0.0, 0.6, 1)   强调
```

**规则**：

- 所有颜色 / 背景 / 边框变化必须有 `--dur-fast` 过渡：`transition: background var(--dur-fast) var(--ease-standard), color var(--dur-fast), border-color var(--dur-fast);`
- 主题切换必须有 `--dur-normal` 颜色插值（丝滑渐变，不是瞬间闪烁）。
- 禁止无意义的弹簧动画（scale bounce 等）。

---

## 8. Shell 布局（App Grid）

Pier-X 顶层是一个四行三列的 grid：

```css
.app {
  display: grid;
  grid-template-rows: var(--titlebar-h) var(--tabbar-h) 1fr var(--statusbar-h);
  grid-template-columns: var(--sidebar-w) 1fr var(--toolstrip-w);
  grid-template-areas:
    "title   title   title"
    "side    tabs    tabs"
    "side    center  right"
    "status  status  status";
}
```

| 区域 | 组件 | 作用 |
|---|---|---|
| title | `TopBar` | 应用标题、全局搜索、主题 / 密度切换、窗口按钮（非 macOS 平台） |
| tabs | `TabBar` | 多后端 tab 切换，带颜色标与关闭按钮 |
| side | `Sidebar` | 两段式：上段（files/servers/snippets tabs）、下段（服务列表） |
| center | 活动 panel | Terminal / Welcome / 面板内容；永不超出 overflow |
| right | `rightzone`（`RightSidebar` + `ToolStrip`） | 当前 tab 的 side-tool panel + 右侧工具列 |
| status | `StatusBar` | branch / user / host / port / notification 计数 |

### 8.1 Resizable

- `ResizeHandle`（4px 宽，`cursor: col-resize`，hover/drag 显示 accent）：位于 sidebar 与 center 之间、center 与 rightpanel 之间。
- 宽度持久化到 `localStorage` 键 `pierx:pane-widths`。
- 约束：sidebar 200–60% 可用宽、rightpanel 260–70% 可用宽。

### 8.2 WelcomeView

当没有活动 tab 时显示：居中 serif italic 大字标题 + 新建 SSH / 打开文件夹 / 文档链接的按钮行。**唯一允许使用 Serif 的位置**。

### 8.3 Overflow 约束

`.center` 永远 `overflow: hidden`；内部 panel 自己管 scroll。禁止顶层出现纵向滚动条。

---

## 9. 共享原子目录（必须使用，禁止自画）

所有 panel / dialog / shell 代码**必须**组合以下原子。在 `components/` 找不到想要的组件时，**先加原子再用**，不要在 panel 内 inline。

### 9.1 Button 三级 + primary .btn

| 组件 | class | 尺寸 | 场景 |
|---|---|---|---|
| `IconButton variant="mini"` | `.mini-btn` | 20×20 | 行内操作、panel-header action、sidebar toolbar |
| `IconButton variant="icon"` | `.icon-btn` | 26×26 | TopBar 全局动作、dialog 关闭（X） |
| `IconButton variant="tool"` / `ToolStripItem` | `.ts-btn` | 32×32 | 仅 ToolStrip 右侧工具列 |
| `.gb-btn` | — | 高 `var(--control-height)`（28–30px） | 次要文字按钮（Cancel、Refresh 等） |
| `.gb-btn.primary` | — | 同上，`--accent` 底色 | 对话框主按钮（Connect / Run / Save） |

**规则**：

- 所有 icon button 透明背景；hover 显示 `--panel-2` 底色。
- `.ts-btn.active` 状态：左侧 2px accent 竖条 + `--accent-dim` 底色 + `--accent` 图标色。
- `.ts-btn.dim`（未检测到服务）：`opacity: 0.32`，hover 升至 0.6。
- Destructive hover 用 `color: var(--neg)`；禁止红底按钮。
- 禁止自写 `<button style={{...}}>`。

### 9.2 PanelHeader

```tsx
<PanelHeader
  icon={Database}
  title="MYSQL"
  meta="warehouse · tunnel :33061"
  actions={<IconButton variant="mini"><MoreH/></IconButton>}
/>
```

- 高度 `var(--panel-header-h)`（34px compact / 38px comfortable）。
- 背景 `var(--surface-2)`；底边 `border-bottom: 1px solid var(--line)`。
- 标题 Mono UPPERCASE `var(--ui-fs-sm)` 600 · tracking 0.06em。
- icon 12px · `--accent` 色。
- meta 文字 Mono 10.5px · `--muted` · 省略号截断。
- 右侧 actions 使用 `mini` variant IconButton，gap `var(--sp-1)`。

**禁止 panel 自绘顶部**；新 panel 一律挂 `PanelHeader`，然后在其下用 `DbConnRow` 或 `section-header` 展开。

### 9.3 状态原子

```tsx
<StatusDot tone="pos|off|warn|neg"/>           // 7×7 圆点 + 2px 外发光
<Badge tone="pos|warn|neg|info|muted">up 18h</Badge>   // 药丸（9×10 内 padding，mono 10.5px）
<Pill>→ :5432</Pill>                            // mono 10.5px pill；tinted 版走 accent
```

- `StatusDot.on`：`background: var(--pos); box-shadow: 0 0 0 2px color-mix(in srgb, var(--pos) 28%, transparent)`
- `Badge.tone="pos"`：`background: var(--pos-dim); color: var(--pos); border: 1px solid color-mix(--pos, transparent 60%)`
- 其他 tone 类推。

### 9.4 ToolStripItem

```tsx
<ToolStripItem
  icon={Database}
  label="MySQL"
  tint="var(--svc-mysql)"
  active={active}
  detected={detected}       // 右上 5×5 --pos 点
  dim={!detected}           // opacity 0.32
  onClick={...}
/>
```

- 32×32 方形；图标 16px；`--ts-tint` 用于 icon 着色。
- active 左侧 2px accent 竖条由 `::before` 绘制。
- detected 小绿点由 `::after` 绘制。

### 9.5 DbConnRow（数据库 / 容器 / 远程服务面板必备）

panel-header 下一行，展示连接元信息：

```
[icon 22×22] warehouse                      [· :33061 pill]
             MySQL 8.0.36 · tunnel over SSH
```

结构：

```tsx
<div className="db-conn-row">
  <span className="db-conn-icon">...</span>
  <div className="db-conn-body">
    <div className="db-conn-name">warehouse</div>
    <div className="db-conn-meta mono">MySQL 8.0.36 · tunnel over SSH</div>
  </div>
  <span className="db-conn-tag mono">:33061</span>
</div>
```

- 大字标题 13px 600 tracking `-0.01em`
- 小字 mono 10.5px `--muted`
- 右侧 tag 端口 / 健康状态

### 9.6 Row 原语

| 组件 | class | 用途 |
|---|---|---|
| `FileRow` | `.file-row` | SFTP / 本地文件浏览器（icon + name + mod + size 四列） |
| `ServerRow` | `.srv-row` | 侧栏连接列表（dot + 两行文字 + auth icon） |
| `DbRow` | `.db-row` / `tr` | 数据库结果表（row-num + cells + hover actions） |
| `CmdPaletteItem` | `.cmdp-item` | 命令面板项（icon + title + shortcut） |
| `ContextMenuItem` | `.ctx-item` | 右键菜单项 |

**共享特性**：

- 高度 `var(--row-h)`（26px / 32px）
- hover：`background: var(--panel-2)`
- selected：`background: var(--accent-dim); color: var(--ink); border-left: 2px solid var(--accent)`
- mono 子列（size / port / time）继承 `--mono`

---

## 10. Dialog 原语

所有对话框共享 `.dlg-*` vocabulary。**禁止**自绘 dialog chrome。

### 10.1 基础结构

```tsx
<div className="cmdp-overlay" onClick={onClose}>
  <div className="dlg dlg--newconn" onClick={stop}>
    <div className="dlg-head">
      <span className="dlg-title">
        <Icons.Plug size={13}/> New SSH Connection
      </span>
      <IconButton variant="mini" onClick={onClose}><X size={12}/></IconButton>
    </div>
    <div className="dlg-body dlg-body--form">...form content...</div>
    <div className="dlg-foot">
      <span className="dlg-foot-hint mono"><span className="kbd">Esc</span> cancel</span>
      <div style={{flex: 1}}/>
      <button className="gb-btn" onClick={onClose}>Cancel</button>
      <button className="gb-btn primary" onClick={onConnect}>Connect</button>
    </div>
  </div>
</div>
```

### 10.2 尺寸修饰符

```
.dlg.small         360px 宽（确认 / 简短输入）
.dlg--newconn      520px 宽（新建连接）
.dlg--settings     820px × 600px（设置页）
.dlg--diff         1120px × 86vh（diff 弹窗）
```

### 10.3 Body 变体

```
.dlg-body                  默认：overflow hidden，block 布局
.dlg-body--form            form 布局：padding var(--sp-4); overflow-y: auto
.dlg-body.dlg-diff-body    diff 布局：padding 0; 内部 dlg-diff-files + dlg-diff-pane flex 分割
```

### 10.4 Form 原语（form dialog 专用）

```
.dlg-form            wrapper；display: flex; flex-direction: column; gap: var(--sp-3)
.dlg-row             单列：label 上 / input 下
.dlg-row2            双列：label-ctrl-hint 的 24-1fr-auto 栅格
.dlg-row-label       sans 11.5px 500 --ink-2
.dlg-input           text input；背景 --panel、border --line-2、focus --accent
.dlg-opts            segmented 行（auth method 切换）
.dlg-opt             segmented 子项；.on 用 --accent-dim 底
.dlg-sel-box         droplist trigger
.dlg-note            小字 hint，mono 10.5px --muted
.dlg-switch          toggle 开关行
.dlg-pane-head       settings 分组标题
.dlg-stepper-unit    数字步进
```

### 10.5 键盘约定

- **Esc** 关闭（overlay click 同等效果）。
- **Enter** 触发主按钮（form 内）；Shift+Enter 在多行输入里换行。
- **Tab** 在 form 内循环 focus，**trap** 在 dialog 内（不能 Tab 出去）。
- 打开时聚焦首个 input（或 title）；关闭时焦点归还 trigger。

### 10.6 设置对话框导航

```tsx
<Fragment>
  <div className="dlg-nav-group">Appearance</div>
  <button className={`dlg-nav-btn${section === "theme" ? " on" : ""}`}>Theme</button>
  <button className="dlg-nav-btn">Density</button>
  <div className="dlg-nav-group">Shortcuts</div>
  ...
</Fragment>
```

- `.dlg-nav-group`：mono uppercase 10px label，不可点击。
- `.dlg-nav-btn`：扁平 row，active 用 `.on`（左侧 2px accent 竖条）。

---

## 11. Diff 原语

Diff 视图出现在 `DiffDialog` 和 `GitPanel` 中，两者共享 class。

### 11.1 Layout

```
.dlg-diff-files          左栏文件列表（仅多文件时显示）
.dlg-diff-pane           右栏 diff 内容（flex 1）
.dlg-diff-scroll         滚动容器；.mono 强制等宽
.dlg-diff-scroll.split   split 模式（两列）
.dlg-diff-scroll.wrap    开启软换行
```

### 11.2 行原语

```
.dlg-diff-line                单行 row
.dlg-diff-line.u-add          unified add → --add-bg tinted
.dlg-diff-line.u-del          unified del → --del-bg tinted
.dlg-diff-line.u-ctx          unified context → 默认色
.dlg-diff-line.s-add          split side add
.dlg-diff-line.s-del          split side del
.dlg-diff-line.s-empty        split 的空行占位
.dlg-diff-ln                  行号列（mono 10.5px --muted）
.dlg-diff-sign                ±/ space 符号列
.dlg-diff-code                代码内容；white-space: pre
```

### 11.3 配色

```
add 背景   color-mix(in srgb, var(--add) 12%, transparent)
add 文字   var(--add)
del 背景   color-mix(in srgb, var(--del) 12%, transparent)
del 文字   var(--del)
```

### 11.4 Hunk 头

`.dlg-diff-hunk-head` — mono 10.5px `--muted`，背景 `--surface-2`，显示 `@@ -10,7 +10,9 @@` 之类的 hunk 元信息。

### 11.5 工具条

`.dlg-diff-toolbar` 内容：unified / split segmented、wrap toggle、whitespace toggle、prev/next 跳跃按钮。位于 `dlg-head` 内右对齐。

---

## 12. Panel 模式

所有 panel 文件位于 `src/panels/`。每个 panel 必须：

1. 以 `PanelHeader` 开头。
2. 紧接 `DbConnRow` 或 section header（取决于语义）。
3. 中段用 panel-specific 布局（`.git-diff`、`.db-editor`、`.rds-tree` 等）。
4. 遵守 `overflow` 约束：panel 根元素 `overflow: hidden`，内部滚动容器自管。

### 12.1 GitPanel（`.git-*`）

```
PanelHeader "GIT · branch:main"
├── section-header "STAGED"    [diff list with diff-line]
├── section-header "CHANGES"   [diff list]
├── composer                   [textarea + commit button]
└── history-strip              [recent commits scroller]
```

- Diff 在这里**嵌入**渲染（非 dialog），使用相同 `.dlg-diff-line` 类。
- 每个文件 row：status letter (M/A/D) + path + stage/unstage 按钮。
- Commit composer 用 `.gb-btn.primary` 触发。

### 12.2 TerminalPanel

```
PanelHeader "TERMINAL · pts/0 · ssh:host"
└── xterm 容器（WebglAddon，主题通过 terminalTheme 注入）
```

- 终端字体：`var(--mono)` 12.5px / line-height 1.45。
- Copy-on-select；paste via Cmd/Ctrl+V；快捷菜单走 `ContextMenu`。
- 禁止在终端区域内叠加其他 DOM layer（影响 WebGL 性能）。

### 12.3 数据库系 Panel（MySQL / Postgres / SQLite）

```
PanelHeader
DbConnRow (instance + DB picker)
├── .db-editor-head (query name + lock + Run button)
├── .db-editor (SQL textarea / CodeMirror)
└── .db-result (results table with .db-row / .db-td-input)
```

- **默认 Read-only**（`.db-lock`）；切换到 writable 需 **显式** 切 lock（UI 变 `--warn` 色，提示风险）。
  改动 read-only 默认**必须**先更新 `docs/PRODUCT-SPEC.md`（见 CLAUDE.md Rule 8）。
- 结果表：`tr.db-row`，hover `--panel-2` 底，row-num 固定列 `--surface-2` 底，actions 隐藏至 hover。
- 双击 cell 进入编辑模式：输入框 `.db-td-input` 出现，2px `--accent` border。
- 新增行：`.db-row-new` 绿色 tint 提示未提交。

### 12.4 RedisPanel（`.rds-*`）

```
PanelHeader
DbConnRow (instance + DB index picker)
└── split pane
    ├── .rds-tree / .rds-keys  (left: hierarchical key tree)
    └── .rds-detail            (right: value editor, type-aware)
```

- Key 类型徽章：`.rds-type-badge`（string / list / hash / set / zset / stream）。
- Delete key：确认 dialog（`.dlg.small`）；不可无确认删除。

### 12.5 DockerPanel（`.docker-*`）

```
PanelHeader
DbConnRow (host + context)
├── container list (cards or rows with status dot)
└── detail pane (logs / stats / exec shell)
```

- 状态 chip 走 `.srv-svc`（docker tint）；运行中用 `StatusDot.on`。
- Destructive 操作（rm -f、kill）必须走 `.dlg.small` 二次确认。

### 12.6 SftpPanel（`.sftp-*`）

```
PanelHeader
toolbar (path breadcrumb + upload / download / mkdir)
└── file list (FileRow list with drag&drop)
```

- Breadcrumb 走 `.crumb` 原语。
- Upload / Download 显示进度条（`.sftp-progress`）。

### 12.7 ServerMonitorPanel（`.mon-*`）

```
PanelHeader
.mon-host (hostname + uptime)
├── 2×2 gauge grid (.gauge: CPU / MEM / Disk / Net)
├── .mon-strip (pill row: os / kernel / arch)
└── .mon-table (top processes)
```

### 12.8 LogViewerPanel（`.log-*`）

```
PanelHeader
toolbar (filter input + level checkboxes + pause/clear)
└── log stream (virtualized list with ANSI coloring)
```

- Level 配色：ERROR → `--neg`，WARN → `--warn`，INFO → `--info`，DEBUG → `--muted`。
- Follow tail 默认开启；滚轮上滚自动关闭 follow。

### 12.9 MarkdownPanel（`.md-*`）

- 只读渲染 `pulldown-cmark` HTML。
- H1–H3 用 sans 600；code block 用 mono 11.5px；表格 `.md-table`；callout `.md-callout` 走 `--info-dim` 底。
- **Welcome 页以外禁止使用 serif**；markdown 本身也不走 serif。

---

## 13. 状态 / 交互约定

### 13.1 State classes

| 状态 | class | 视觉 |
|---|---|---|
| hover | `:hover` | `--panel-2` 底（大多数 row / button） |
| active/selected | `.on` / `.active` / `.sel` / `.selected` | `--accent-dim` 底 + `--ink` 色 + 左边 2px accent 竖条（list）或底部 2px accent 下划线（tab） |
| focus | `:focus-visible` | 2px `--accent` outline + 1px offset |
| pressed | `:active` | `--bg-active`（透明黑叠加） |
| disabled | `[disabled]` / `.dim` | `opacity: 0.32`（或 0.45）；cursor: default；禁止 pointer-events |
| read-only | `.db-lock` 非 `.writable` | mono label + lock icon + `--muted` 色 |
| unsaved/editing | `.db-td-input`（双击 cell） | 2px `--accent` border |
| error | `.is-error` / `.neg` | `--neg` 文字 + `--neg-dim` 底（当需要填充时） |
| warning | `.is-warn` / `.warn` | `--warn` + `--warn-dim` |

### 13.2 过渡

```css
* {
  transition:
    background var(--dur-fast) var(--ease-standard),
    color var(--dur-fast) var(--ease-standard),
    border-color var(--dur-fast) var(--ease-standard);
}
```

仅这三类属性走全局过渡；transform / opacity 按需单独写。

### 13.3 滚动条

全局 `::-webkit-scrollbar`：

```css
::-webkit-scrollbar { width: 10px; height: 10px; }
::-webkit-scrollbar-thumb {
  background: var(--line-2);
  border-radius: 5px;
  border: 2px solid transparent;
  background-clip: content-box;
}
::-webkit-scrollbar-thumb:hover {
  background: var(--line-3);
  background-clip: content-box;
}
```

禁止 panel 内自定义滚动条样式。

---

## 14. 图标

- **源**：`lucide-react`（参考 `src/components/icons/`）；原型 `pier-x-copy/src/icons.jsx` 是视觉基准但不是代码来源。
- **stroke**：1.75（Lucide 默认），永不改动。
- **默认 size**：`11–13px`（UI）、`16px`（toolstrip）、`9–12px`（按钮 inline）、`18px`（welcome / hero）。
- **颜色**：通过 `currentColor` 继承父元素 color；**禁止**在 JSX 内 `stroke="#..."`。
- 需要 tint 时使用 CSS 变量（`style={{color: "var(--svc-mysql)"}}`）或 class (`--ts-tint` 变量）。

---

## 15. 无障碍

- 所有可点击元素有 `title` 或 `aria-label`（tooltip 兜底）。
- 焦点环永远可见（`:focus-visible`），禁止 `outline: none` 无替代。
- Dialog 打开时 trap focus；按 Esc 关闭；overlay click 关闭。
- **颜色不是唯一信息源**：状态永远用 icon + 文字 + 颜色三者组合（`.db-badge` 的 dot + 文字，`.srv-row` 的 StatusDot + 名字）。
- Terminal / 表格最小字号 11px。禁止再小。
- 键盘操作覆盖：
  - Cmd/Ctrl+K 打开 command palette；
  - Cmd/Ctrl+N 新建 tab；
  - Cmd/Ctrl+W 关闭 tab；
  - Esc 取消 / 关闭 dialog；
  - Arrow keys 在 list / palette 内导航。

---

## 16. Review Gate（禁止清单）

任何 PR 若命中下列任意一条必须拒绝：

1. 在 `src/shell/*` / `src/panels/*` / `src/components/*` 的 CSS / TSX 里出现 hex / rgb / rgba / hsl 字面量（`tokens.css` / `fonts.css` 除外）。
2. 在上述目录出现 `font-size: <number>px` 字面量而非 `var(--size-*)` / `var(--ui-fs*)`。
3. 在 JSX 里 hardcode 字体家族字符串（`"IBM Plex Sans"`、`"Inter"`、`"JetBrains Mono"`），而非 `var(--sans)` / `var(--mono)` / `var(--serif)`。
4. 自写 `<button style={...}>` 或自绘 panel 顶部 header，而不用 `IconButton` / `.gb-btn` / `PanelHeader`。
5. Dialog 不走 `.dlg-head` / `.dlg-body` / `.dlg-foot` 三段式结构。
6. 同一视图引入第二个 accent 色（同一时刻只允许一个 `--accent`）。
7. 使用 Serif 在 Welcome 页以外的任何位置。
8. 使用实色黑线 / 白线边框（必须走 `--line / --line-2 / --line-3`）。
9. 数据库面板默认 writable 而非 read-only（改变默认必须先改 `docs/PRODUCT-SPEC.md`）。
10. 添加 / 移除 / 重用途 toolstrip 条目，或改变 panel 默认 `rightTool`，未先更新 `docs/PRODUCT-SPEC.md`。
11. 渲染路径（JSX body、`useMemo` deps）同步调用 `invoke`（违反 CLAUDE.md Rule 5）。
12. 引入 Qt / QML / CMake / Corrosion / `pier-ui-gpui` 任何 artefact。

---

## 17. 文件位置速查

| 关注点 | 文件 |
|---|---|
| 原型视觉基准（只读参考） | `pier-x-copy/index.html`、`pier-x-copy/Page.html`、`pier-x-copy/src/*.jsx` |
| CSS 变量定义（单一真源） | `src/styles/tokens.css` |
| 共享原子 class | `src/styles/atoms.css` |
| 原型迁移类（dialog / diff 等 `.dlg-*` / `.cmdp-*`） | `src/styles/pier-x.css` |
| Shell chrome 布局 | `src/styles/shell.css` |
| Git 面板 scoped 样式 | `src/styles/git-panel.css` |
| 字体 | `src/styles/fonts.css` |
| 原子 React 组件 | `src/components/` |
| Shell 组件（TopBar / Sidebar / TabBar / StatusBar / WelcomeView / Dialogs） | `src/shell/` |
| Panels | `src/panels/` |
| Zustand stores（UI 状态） | `src/stores/` |
| Tauri 命令 wrapper（typed invoke） | `src/lib/` |
| i18n | `src/i18n/` |

---

## 18. 与上游文档的关系

- `CLAUDE.md` 定义代码结构约束（module layout、state in stores、Tauri IPC 边界、render-is-paint-only）。**结构 vs 视觉冲突**：此文件赢视觉（颜色 / 字号 / 组件）；CLAUDE.md 赢结构（文件放哪、怎么 invoke）。
- `AGENTS.md` 是简化版工程规则，偏构建流程。
- `docs/PRODUCT-SPEC.md` 是功能边界的唯一真源（哪些 panel、哪些 tool、默认安全姿态）。**视觉实现不能修改功能边界**——先改 SPEC，再改视觉。
- `docs/ROADMAP.md` 是节奏，不是规范。
- `.agents/skills/pier-design-system/extracted/` 下的 `linear.md` / `warp.md` / `raycast.md` 等是**灵感引用**，不是规则。这里的规则已从它们中提炼。

**优先级**：PRODUCT-SPEC（功能） > 本 SKILL（视觉） > CLAUDE.md（结构） > AGENTS.md（简化） > ROADMAP（节奏）。
