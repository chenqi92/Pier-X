---
name: pier-design-system
description: Pier-X 视觉与交互的唯一设计标准。任何 Pier-X UI 工作 — 颜色、字体、间距、组件、主题切换、动画 — 都必须遵守这套规范。当你为 Pier-X 生成 QML 代码、定义 Theme Singleton、选择颜色、设定字号、构建可复用组件、或决定任何视觉细节时，应用这套 tokens。目标：在 Qt 6 + QML 上做出 IntelliJ 级别的工程化深色/浅色 IDE 体验，跨 macOS 与 Windows。
---

# Pier-X Design System

> **Engineered darkness, instrument precision.**
> 工程化的暗色，仪器级的精度。JetBrains 级 IDE 体验，专为跨平台终端管理工具设计。
>
> 综合自 Linear（luminance stacking）+ Warp（终端克制）+ Raycast（macOS 原生深度）+ JetBrains Darcula（熟悉的 IDE 心智）。
> **实现目标：Qt 6 + QML**，亮 / 暗双主题，macOS + Windows。

---

## 1. 五条不可妥协的原则

| # | 原则 | 含义 |
|---|---|---|
| 1 | **Darkness is the medium, not a theme** | 深色是原生媒介，浅色是镜像。层级通过**亮度叠层**（背景透明度阶梯）传达，而非颜色变化 |
| 2 | **Single chromatic accent** | 全系统**只有一个**强调色：默认 `#4AA3FF`（蓝）。accent 可切换到 green / amber / violet / coral，但**同一时刻只有一个**。状态色用克制的绿/黄/红。**禁止装饰性用色** |
| 3 | **Semi-transparent borders, never solid** | 深色下边框允许用实色暗灰 `#242a36 / #2e3542 / #3a4254`（三阶），浅色下 `#e4e0d4 / #d6d2c4 / #c6c1b0`。**禁止高对比黑线或白线** —— 那样看起来机械、过时 |
| 4 | **IBM Plex for everything** | Sans 用于 UI、Mono 用于代码/终端/路径/IP/端口、Serif italic 仅用于 welcome 编辑器化大字。**500 是签名权重**，600 用于小号 mono uppercase 标签 |
| 5 | **Density over spectacle** | 这是 IDE 级工具，不是营销页。**12px compact / 13px comfortable** 基础文字、4px 间距栅格、34px 面板头、不要巨型标题、不要装饰渐变 |

> **检查任何 PR 时反问**：这一改动是否违反了上面五条之一？如果是，**不要合并**。

---

## 2. 颜色 Tokens

### 2.1 深色主题（默认）

```
背景层（luminance stacking — 越高的层 = 越亮的背景）
─────────────────────────────────────────────
--bg               #0E1116   主窗口最深背景 (canvas)
--surface          #12161D   停靠面板、侧边栏
--surface-2        #171C25   tab bar、panel header 底色、状态栏
--panel            #1A202B   卡片、对话框、抬升表面
--panel-2          #222937   hover 态、hover chip、二级表面
--elev             #252D3D   popover、菜单、tooltip
--bg-hover         rgba(255,255,255,0.05)   hover 叠加
--bg-active        rgba(255,255,255,0.08)   按下/活跃

文本
─────────────────────────────────────────────
--ink              #E5E9F0   主要文字（不是纯白！）
--ink-2            #B9C1CC   次要文字（描述、标签）
--muted            #747D8B   弱化文字（占位、metadata）
--dim              #4E5663   禁用、分隔符
--accent-ink       #0A1420   accent 按钮上的文字

边框（三阶灰 + 唯一的 accent 焦点）
─────────────────────────────────────────────
--line             #242A36   默认（输入框、卡片、面板）
--line-2           #2E3542   常规（突出的边界）
--line-3           #3A4254   强（重要分隔、按钮边框）
--accent (focus)   #4AA3FF   键盘焦点、强调

强调色（**全系统唯一彩色**，可切换 accent variant）
─────────────────────────────────────────────
--accent           #4AA3FF   默认 · IntelliJ Remix 蓝
--accent-hover     #6EB6FF   hover
--accent-dim       #1E3A5C   accent 背景填充（muted）
--accent-subtle    rgba(74,163,255,0.08)   极淡 accent 背景

状态色（克制使用，仅用于状态指示；两主题共享）
─────────────────────────────────────────────
--pos              #3DD68C   运行中、成功
--warn             #FFB547   警告
--neg              #FF5A5F   错误
--info             #7AA2F7   信息 (辅助蓝，与 accent 区分)

Git diff / 语法高亮（与状态色同源但命名分离，语义更清晰）
─────────────────────────────────────────────
--add              #3DD68C
--del              #FF5A5F
--mod              #FFB547
```

### 2.2 浅色主题（深色的镜像）

```
背景层（暖象牙色，避免纯白刺眼）
─────────────────────────────────────────────
--bg               #F5F3EE   主窗口最深背景
--surface          #FBFAF5   停靠面板、侧边栏
--surface-2        #F3F1EA   tab bar、面板头底色
--panel            #FFFFFF   卡片、对话框、抬升表面
--panel-2          #F9F7F1   hover、二级表面
--elev             #FFFFFF   popover（靠阴影区分层级）

文本
─────────────────────────────────────────────
--ink              #14171D   不是纯黑！
--ink-2            #384050
--muted            #6E7585
--dim              #9AA0AD
--accent-ink       #FFFFFF

边框（浅色下允许低对比实色边，与深色下的三阶对应）
─────────────────────────────────────────────
--line             #E4E0D4
--line-2           #D6D2C4
--line-3           #C6C1B0
--accent           #4AA3FF   同深色主题

强调色 / 状态色：与深色主题数值相同（保持品牌一致）
仅 --accent-dim 改为高亮版 #D6E6FA（浅色下需要可见的 accent 填充）
```

### 2.3 Accent variants（可切换的品牌色）

用户可在 Settings → Appearance 里切 accent：

```
blue   (default)  --accent: #4AA3FF   --accent-dim: #1E3A5C (dark) / #D6E6FA (light)
green             --accent: #3DD68C   --accent-dim: #17402C / #CDEEDB
amber             --accent: #FFB547   --accent-dim: #3E2C14 / #F7E4BF
violet            --accent: #B48CFF   --accent-dim: #2E2142 / #E4D8FA
coral             --accent: #FF7A59   --accent-dim: #3E1E14 / #F7DCCF
```

切换时 `document.documentElement.dataset.accent` 变化，tokens.css 中 `[data-accent="..."]` 覆写规则生效。

### 2.4 终端 ANSI 16 色调色板（两种主题通用）

终端区域使用专门的 ANSI 调色板，与 UI chrome 解耦：

```
        Normal      Bright
black   #1c1e22     #5a5e66
red     #ff5a5f     #ff8593
green   #3dd68c     #7fcf85
yellow  #ffb547     #ffc15c
blue    #4aa3ff     #7cb9ff
magenta #c49eff     #d894ed
cyan    #56e0c8     #7fc8d1
white   #b9c1cc     #e5e9f0
```

---

## 3. 字体 Tokens

### 3.1 字体家族（IBM Plex 全家桶）

```
UI:       IBM Plex Sans      (开源 · OFL)  --sans
Mono:     IBM Plex Mono      (开源 · OFL)  --mono
Serif:    IBM Plex Serif     (开源 · OFL)  --serif   ← 仅用于 Welcome 大字 italic

Fallback: system-ui → Inter → Segoe UI → SF Pro Text
Mono fb:  JetBrains Mono → SF Mono → Consolas → ui-monospace
```

> 通过 Google Fonts 在 `index.html` 加载，离线时走系统 fallback。生产构建建议 bundle IBM Plex 到应用资源里，避免网络抖动。

### 3.2 类型阶梯

**注意：IDE 工具的基础 UI 文字是 compact 12px / comfortable 13px。** 不是营销页的 16px。

| Role | Font | Size | Weight | Line Height | 用途 |
|---|---|---|---|---|---|
| Welcome hero | Serif italic | 44px | 400 | 1.10 | Welcome 页唯一 serif 使用点 |
| H1 | Sans | 24px | 600 | 1.30 | 设置页主标题 |
| H2 | Sans | 20px | 600 | 1.35 | 对话框标题 |
| H3 | Sans | 16px | 600 | 1.40 | 卡片标题、分区标题 |
| Body Large | Sans | 14px | 400 | 1.50 | 主要阅读文本、dialog body |
| **Body** | **Sans** | **12px (c) / 13px (cf)** | **400** | **1.45** | **默认 UI 文字（最常用）** |
| UI Label | Sans | 12px | 500 | 1.0 | 按钮、tab、工具栏 |
| Caption | Sans | 11px | 500 | 1.40 | 辅助标签、列头 |
| Small | Sans | 11px | 400 | 1.40 | 提示 |
| Metadata | Mono | 10.5px | 400 | 1.4 | 状态栏、meta、端口、IP、时间戳 |
| Mono Code | Mono | 12.5px | 400 | 1.45 | 终端、代码、SQL |
| Mono Small | Mono | 11.5px | 400 | 1.40 | 内联代码、路径 |
| Panel Title | Mono | 11.5px | 600 | 1.0 | 所有 PanelHeader 标题（UPPERCASE, tracking 0.06em） |
| Section Header | Mono | 10.5px | 600 | 1.0 | 面板内小分区标题（UPPERCASE, tracking 0.08em） |

### 3.3 字体规则

- **500 是签名权重**（IBM Plex 的 medium）。所有 UI 标签、按钮、菜单项默认 500。Body 用 400，强调/面板标题用 600。允许 700 仅用于极少数 dialog H1。
- **PanelHeader 与 SectionHeader 一律 Mono UPPERCASE**（`text-transform: uppercase; letter-spacing: 0.06–0.08em`）。这是 Pier-X 的"工程图"质感标志。
- **代码 / 路径 / IP / 端口 / 命令 / 时间戳 / 大小 / 列头 必须用 Mono**。任何「机器可读」或「等宽对齐」的内容都是 Mono 范畴。
- **Serif italic 仅用于 Welcome 页 hero**。禁止在其他位置混入 serif。
- **不要混入第四种字体家族**。

---

## 4. 间距 Tokens（4px 栅格）

IDE 是高密度界面，**用 4px 栅格，不是 8px**。JetBrains 全套 IDE 都用 4px 增量。

```
spacing.0      0px
spacing.0_5    2px    （图标内部微调）
spacing.1      4px    （最小 gap）
spacing.1_5    6px    （inline 元素间）
spacing.2      8px    （组件内 padding 标准）
spacing.3      12px   （组件之间 gap）
spacing.4      16px   （区块内边距）
spacing.5      20px   （次级区块间）
spacing.6      24px   （主要区块间）
spacing.8      32px   （大区块）
spacing.10     40px
spacing.12     48px   （hero 间距）
```

**常用组合**：
- 按钮内边距：`spacing.2 spacing.3`（8px 12px）
- 输入框内边距：`spacing.2 spacing.3`
- 卡片内边距：`spacing.4`
- 工具栏行高：32px（精确，不是 spacing 倍数）
- 菜单项高度：28px
- 列表项高度：24px（紧凑）/ 28px（标准）

---

## 5. 圆角 Tokens

```
radius.none    0
radius.xs      2px    （内联 badge、status dot）
radius.sm      4px    （按钮、输入框、tab）
radius.md      6px    （卡片、popover、菜单）
radius.lg      8px    （对话框）
radius.xl      12px   （大型 panel）
radius.pill    9999px （状态药丸）
radius.circle  50%    （图标按钮、头像）
```

**默认是 `radius.sm` (4px)**。IDE 风格不是 macOS 那种大圆角，是更克制的 4–6px。

---

## 6. 阴影与高度 Tokens

### 6.1 深度模型

Pier-X 的深度系统遵守 Linear 的「luminance stacking」+ Raycast 的「macOS native multi-layer」混合：

| 层级 | 实现 | 用途 |
|---|---|---|
| **L0 Flat** | 无阴影，`bg.canvas` | 主窗口背景 |
| **L1 Panel** | `bg.panel` | 停靠面板、侧边栏（仅靠背景色区分） |
| **L2 Surface** | `bg.surface` + `border.subtle` | 卡片、对话框 |
| **L3 Elevated** | `bg.elevated` + `border.default` + `shadow.soft` | 抬升的面板 |
| **L4 Popover** | `bg.elevated` + `shadow.popover` | 下拉菜单、tooltip、command palette |
| **L5 Modal** | `bg.elevated` + `shadow.modal` + 背景遮罩 | 模态对话框 |

### 6.2 阴影定义（深色主题）

```
shadow.soft (L3)
  0 1px 2px rgba(0,0,0,0.20)
  0 2px 6px rgba(0,0,0,0.16)

shadow.popover (L4)  ← Raycast 风格多层
  0 0 0 1px rgba(0,0,0,0.40)            ← 外环
  0 8px 24px rgba(0,0,0,0.32)            ← 主阴影
  0 2px 8px rgba(0,0,0,0.24)             ← 中阴影
  inset 0 1px 0 rgba(255,255,255,0.05)   ← 顶部高光（关键！）

shadow.modal (L5)
  0 0 0 1px rgba(0,0,0,0.50)
  0 24px 64px rgba(0,0,0,0.48)
  0 8px 24px rgba(0,0,0,0.32)
  inset 0 1px 0 rgba(255,255,255,0.06)
```

---

## 8. 共享原子（Pier-X React 实现）

> Qt/QML 外壳已归档 —— 当前前端是 **Tauri 2 + React 19 + TypeScript**，源码在 `pier-ui-tauri/src/`。
> tokens 定义在 `src/styles/tokens.css`，共享原子类在 `src/styles/atoms.css`，React 组件在 `src/components/`。

### 8.1 按钮三级（一定要选一个，不自己画新的）

| 组件 | class | 尺寸 | 场景 |
|---|---|---|---|
| `IconButton variant="mini"` | `.mini-btn` | 20×20 | 行内操作、panel-header action、sidebar toolbar 按钮 |
| `IconButton variant="icon"` | `.icon-btn` | 26×26 | TopBar 全局动作、dialog header 关闭 |
| `IconButton variant="tool"` / `ToolStripItem` | `.ts-btn` | 32×32 | 仅 ToolStrip 右侧工具列 |
| `.btn` / `.btn.is-primary` / `.btn.is-ghost` / `.btn.is-danger` / `.btn.is-compact` | `.btn` | 高 `--control-height` | 表单、对话框底部、commit composer |

`active` 状态统一 `background: var(--accent-dim); color: var(--accent);`。destructive hover 用 `color: var(--neg)`。

### 8.2 PanelHeader（每个右侧工具面板的唯一顶部）

```tsx
<PanelHeader
  icon={Database}
  title="MYSQL"
  meta="warehouse · tunnel :33061"
  actions={<IconButton variant="mini"><MoreH/></IconButton>}
/>
```

- 高度 `var(--panel-header-h)` (34px compact / 38px comfortable)
- 标题 Mono UPPERCASE `var(--ui-fs-sm)` weight 600 · tracking 0.06em
- icon 12px · accent 色
- meta 文字 Mono 10.5px · `var(--muted)` · 省略号截断
- 不允许面板自绘顶部；新面板一律挂 PanelHeader，然后在其下用 `.db-conn-row` 或 `.section-header` 继续展开

### 8.3 状态原子

- `<StatusDot tone="pos|off|warn|neg"/>` — 7px 状态圆点（服务器在线、端口通/不通）
- `<Badge tone="pos|warn|neg|info|muted">up 18h</Badge>` — 容器 / 查询状态 pill
- `<Pill>→ :5432</Pill>` — mono 10.5px pill，默认中性色；`tinted` 版走 accent

### 8.4 Toolstrip item

所有 toolstrip 按钮走 `<ToolStripItem icon={...} label="MySQL" active detected dim onClick={...}/>`：

- active 左侧 2px 竖条 accent 指示
- detected 右上 5px 绿点
- dim 未检测到的工具 opacity 0.32

### 8.5 DbConnRow（数据库/容器/远程服务面板必备）

panel-header 下一行，展示连接元信息：

```
[icon 22×22] warehouse                      [· :33061 pill]
             MySQL 8.0.36 · tunnel over SSH
```

- 大字标题 13px weight 600 tracking -0.01em
- 小字 Mono 10.5px muted
- 右侧 `.db-conn-tag` 显示端口 / 健康状态

### 8.6 禁止清单（Review Gate）

拒绝以下 PR：

1. 自写 `<button style={...}>`、不走 `IconButton` 或 `.btn` 原子
2. 自写 panel 顶部 header，而不用 `<PanelHeader/>`
3. 在 `src/panels/*` / `src/shell/*` / `src/components/*` 的 CSS 里出现 hex/rgb/rgba 字面量（tokens.css 除外）
4. 在上述目录出现 `font-size: <number>px` 字面量，而不用 `--size-*` / `--ui-fs-*`
5. 在页面里混入第二个 accent 色
6. 用 Inter / JetBrains Mono / 其他 sans/mono family 覆盖 `var(--sans) / var(--mono)`
7. 用 Serif 在 Welcome 页以外的任何位置
8. 用 `border: 1px solid #xxx`（高对比黑或白实线）

---

## Implementation Protocol (legacy · Qt/QML 已归档，保留参考)

> **当前有效的实现协议见 §8**。以下内容来自 Qt 阶段，仅作历史参考。

Pier-X must not style pages by composing raw Qt controls ad hoc. All new UI work must follow this order:

1. Use tokens from `pier-ui-qt/qml/Theme.qml`
2. Prefer an existing control in `pier-ui-qt/qml/components/`
3. If no fitting control exists, create a new reusable component in `qml/components/`
4. Only then use that component in feature pages

### Foundation control whitelist

Use these controls by default:

- Buttons: `PrimaryButton`, `GhostButton`, `IconButton`
- Inputs: `PierTextField`, `PierTextArea`, `PierSearchField`, `PierComboBox`, `PierScrollView`, `ToggleSwitch`, `PierSlider`
- Surfaces: `Card`, `ToolPanelSurface`, `ModalDialogShell`, `PopoverPanel`
- Utility: `SegmentedControl`, `StatusPill`, `PierToolTip`, `PierMenuItem`, `PierScrollBar`

### Forbidden in feature pages

Do not instantiate these directly in page/view/dialog files:

- `Popup`
- `TextField`
- `TextArea`
- `TextInput`
- `ScrollView`
- `ScrollBar`
- page-local menu row rectangles
- one-off slider styling

### Allowed exceptions

- Native application menu bar entries may use `MenuBar` and `MenuItem`
- Foundation wrapper components may internally use raw Qt controls where necessary:
  - `PopoverPanel.qml`
  - `PierComboBox.qml`
  - `PierTextArea.qml`
  - `PierTextField.qml`
  - `PierSearchField.qml`
  - `PierScrollView.qml`
  - `PierSlider.qml`
  - `PierScrollBar.qml`

### Review gate

Reject any UI change that:

- introduces new page-local control styling already covered by a foundation control
- adds a second accent color
- uses a solid dark-on-dark border
- uses oversized spacing or marketing-page typography

### 6.3 阴影定义（浅色主题）

```
shadow.soft
  0 1px 2px rgba(15,20,30,0.06)
  0 2px 6px rgba(15,20,30,0.08)

shadow.popover
  0 0 0 1px rgba(15,20,30,0.08)
  0 8px 24px rgba(15,20,30,0.12)
  0 2px 8px rgba(15,20,30,0.08)

shadow.modal
  0 0 0 1px rgba(15,20,30,0.10)
  0 24px 64px rgba(15,20,30,0.20)
  0 8px 24px rgba(15,20,30,0.12)
```

> **核心规则**：深色主题的阴影必须包含 `inset 0 1px 0 rgba(255,255,255,0.05)` 顶部高光。这是 Raycast 的 macOS 原生质感的来源 —— 让 popover 看起来像玻璃面板而不是平面色块。

---

## 7. 动画 Tokens

```
duration.instant   0ms
duration.fast      120ms   （hover、focus、颜色变化）
duration.normal    200ms   （主题切换、状态转换）
duration.slow      320ms   （面板滑入、模态进入）
duration.slower    480ms   （大型布局变化）

easing.standard    cubic-bezier(0.4, 0.0, 0.2, 1)   ← 默认
easing.decelerate  cubic-bezier(0.0, 0.0, 0.2, 1)   ← 入场
easing.accelerate  cubic-bezier(0.4, 0.0, 1.0, 1)   ← 出场
easing.sharp       cubic-bezier(0.4, 0.0, 0.6, 1)   ← 强调
```

**规则**：所有颜色、背景、边框变化必须有 `duration.fast` 过渡。主题切换必须有 `duration.normal` 颜色插值（这是 Pier-X 与平庸应用的关键差异 —— 主题切换是丝滑的渐变，不是瞬间闪烁）。

---

## 8. QML Theme Singleton 实现

**这是 Pier-X UI 的根。所有 QML 组件必须从这里取色和取尺寸**。

文件位置：`pier-ui-qt/qml/theme/Theme.qml`

```qml
pragma Singleton
import QtQuick

QtObject {
    id: theme

    // ─────────────────────────────────────────
    // 主题切换
    // ─────────────────────────────────────────
    property bool dark: true   // 默认深色

    // 跟随系统（在 main.cpp 里连接 QStyleHints::colorSchemeChanged）
    function followSystem(scheme) {
        dark = (scheme === Qt.ColorScheme.Dark)
    }

    // ─────────────────────────────────────────
    // 颜色 — 背景层
    // ─────────────────────────────────────────
    readonly property color bgCanvas:    dark ? "#0e0f11" : "#fbfcfd"
    readonly property color bgPanel:     dark ? "#16181b" : "#f6f7f9"
    readonly property color bgSurface:   dark ? "#1c1e22" : "#ffffff"
    readonly property color bgElevated:  dark ? "#22252a" : "#ffffff"
    readonly property color bgHover:     dark ? Qt.rgba(1,1,1,0.04) : Qt.rgba(0,0,0,0.04)
    readonly property color bgActive:    dark ? Qt.rgba(1,1,1,0.06) : Qt.rgba(0,0,0,0.06)
    readonly property color bgSelected:  Qt.rgba(53/255, 116/255, 240/255, dark ? 0.16 : 0.10)

    // ─────────────────────────────────────────
    // 颜色 — 文本
    // ─────────────────────────────────────────
    readonly property color textPrimary:    dark ? "#e8eaed" : "#1e1f22"
    readonly property color textSecondary:  dark ? "#b4b8bf" : "#454850"
    readonly property color textTertiary:   dark ? "#868a91" : "#6c707e"
    readonly property color textDisabled:   dark ? "#5a5e66" : "#a7a9b0"
    readonly property color textInverse:    dark ? "#16181b" : "#ffffff"

    // ─────────────────────────────────────────
    // 颜色 — 边框（永远半透明！）
    // ─────────────────────────────────────────
    readonly property color borderSubtle:   dark ? Qt.rgba(1,1,1,0.05) : Qt.rgba(0,0,0,0.06)
    readonly property color borderDefault:  dark ? Qt.rgba(1,1,1,0.09) : Qt.rgba(0,0,0,0.10)
    readonly property color borderStrong:   dark ? Qt.rgba(1,1,1,0.14) : Qt.rgba(0,0,0,0.18)
    readonly property color borderFocus:    "#3574f0"

    // ─────────────────────────────────────────
    // 颜色 — 强调（全系统唯一彩色）
    // ─────────────────────────────────────────
    readonly property color accent:         "#3574f0"
    readonly property color accentHover:    "#4f8aff"
    readonly property color accentMuted:    Qt.rgba(53/255, 116/255, 240/255, 0.16)
    readonly property color accentSubtle:   Qt.rgba(53/255, 116/255, 240/255, 0.08)

    // ─────────────────────────────────────────
    // 颜色 — 状态
    // ─────────────────────────────────────────
    readonly property color statusSuccess:  "#5fb865"
    readonly property color statusWarning:  "#f0a83a"
    readonly property color statusError:    "#fa6675"
    readonly property color statusInfo:     "#3574f0"

    // ─────────────────────────────────────────
    // 字体
    // ─────────────────────────────────────────
    readonly property string fontUi:    "Inter"
    readonly property string fontMono:  "JetBrains Mono"

    // 字号
    readonly property int sizeDisplay:  32
    readonly property int sizeH1:       24
    readonly property int sizeH2:       20
    readonly property int sizeH3:       16
    readonly property int sizeBodyLg:   14
    readonly property int sizeBody:     13   // ← 默认
    readonly property int sizeCaption:  12
    readonly property int sizeSmall:    11

    // 权重（Inter Variable 支持小数权重）
    readonly property int weightRegular:   400
    readonly property int weightMedium:    510   // ← 签名
    readonly property int weightSemibold:  590

    // ─────────────────────────────────────────
    // 间距 (4px 栅格)
    // ─────────────────────────────────────────
    readonly property int sp0:    0
    readonly property int sp0_5:  2
    readonly property int sp1:    4
    readonly property int sp1_5:  6
    readonly property int sp2:    8
    readonly property int sp3:    12
    readonly property int sp4:    16
    readonly property int sp5:    20
    readonly property int sp6:    24
    readonly property int sp8:    32
    readonly property int sp10:   40
    readonly property int sp12:   48

    // ─────────────────────────────────────────
    // 圆角
    // ─────────────────────────────────────────
    readonly property int radiusXs:    2
    readonly property int radiusSm:    4
    readonly property int radiusMd:    6
    readonly property int radiusLg:    8
    readonly property int radiusXl:    12
    readonly property int radiusPill:  9999

    // ─────────────────────────────────────────
    // 动画
    // ─────────────────────────────────────────
    readonly property int durFast:     120
    readonly property int durNormal:   200
    readonly property int durSlow:     320
    readonly property int easingType:  Easing.OutCubic   // 标准缓动
}
```

**注册 Singleton**（`qmldir`）：

```
module Pier.Theme
singleton Theme 1.0 Theme.qml
```

**使用**：

```qml
import Pier.Theme

Rectangle {
    color: Theme.bgPanel
    border.color: Theme.borderSubtle
    border.width: 1
    radius: Theme.radiusSm

    // 主题切换的丝滑过渡
    Behavior on color { ColorAnimation { duration: Theme.durNormal; easing.type: Theme.easingType } }
    Behavior on border.color { ColorAnimation { duration: Theme.durNormal; easing.type: Theme.easingType } }
}
```

**切换主题**：`Theme.dark = !Theme.dark` —— 全局所有绑定带过渡动画自动更新。

---

## 9. 组件 Recipes

### 9.1 Primary Button（强调按钮）

```qml
Rectangle {
    implicitHeight: 28
    implicitWidth: label.implicitWidth + Theme.sp3 * 2
    color: hovered ? Theme.accentHover : Theme.accent
    radius: Theme.radiusSm

    property bool hovered: mouseArea.containsMouse
    Behavior on color { ColorAnimation { duration: Theme.durFast } }

    Text {
        id: label
        anchors.centerIn: parent
        text: "Connect"
        font.family: Theme.fontUi
        font.pixelSize: Theme.sizeBody
        font.weight: Theme.weightMedium
        color: Theme.textInverse
    }

    MouseArea {
        id: mouseArea
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
    }
}
```

### 9.2 Ghost Button（次级按钮）

```qml
Rectangle {
    implicitHeight: 28
    color: hovered ? Theme.bgHover : "transparent"
    border.color: Theme.borderDefault
    border.width: 1
    radius: Theme.radiusSm
    // ... (类似 Primary，文字用 Theme.textPrimary)
}
```

### 9.3 Input Field（输入框）

```qml
Rectangle {
    implicitHeight: 28
    color: Theme.bgSurface
    border.color: input.activeFocus ? Theme.borderFocus : Theme.borderDefault
    border.width: 1
    radius: Theme.radiusSm

    Behavior on border.color { ColorAnimation { duration: Theme.durFast } }

    TextInput {
        id: input
        anchors.fill: parent
        anchors.leftMargin: Theme.sp2
        anchors.rightMargin: Theme.sp2
        verticalAlignment: TextInput.AlignVCenter
        font.family: Theme.fontUi
        font.pixelSize: Theme.sizeBody
        color: Theme.textPrimary
        selectionColor: Theme.accentMuted
        selectedTextColor: Theme.textPrimary
    }
}
```

### 9.4 Card / Panel（卡片）

```qml
Rectangle {
    color: Theme.bgSurface
    border.color: Theme.borderSubtle
    border.width: 1
    radius: Theme.radiusMd

    // 内容用 Theme.sp4 padding
}
```

### 9.5 Status Pill（状态药丸）

```qml
Rectangle {
    implicitHeight: 18
    implicitWidth: row.implicitWidth + Theme.sp2 * 2
    color: Theme.bgSurface
    border.color: Theme.borderSubtle
    border.width: 1
    radius: Theme.radiusPill

    Row {
        id: row
        anchors.centerIn: parent
        spacing: Theme.sp1
        Rectangle {   // 状态点
            width: 6; height: 6
            radius: 3
            color: Theme.statusSuccess
            anchors.verticalCenter: parent.verticalCenter
        }
        Text {
            text: "Running"
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeCaption
            font.weight: Theme.weightMedium
            color: Theme.textSecondary
        }
    }
}
```

### 9.6 Tooltip / Popover（浮层）

```qml
Rectangle {
    color: Theme.bgElevated
    border.color: Theme.borderDefault
    border.width: 1
    radius: Theme.radiusMd

    // 多层阴影需要在 Qt 6 用 MultiEffect 或 layer.effect
    layer.enabled: true
    layer.effect: MultiEffect {
        shadowEnabled: true
        shadowColor: "#000000"
        shadowOpacity: 0.32
        shadowBlur: 1.0
        shadowVerticalOffset: 8
    }
}
```

### 9.7 Tab（标签）

```qml
Rectangle {
    implicitHeight: 32
    implicitWidth: label.implicitWidth + Theme.sp4 * 2
    color: active ? Theme.bgSurface : (hovered ? Theme.bgHover : "transparent")
    radius: 0   // tab 通常无圆角，靠下边框区分

    Rectangle {   // 活跃 tab 的底部蓝条
        anchors.bottom: parent.bottom
        anchors.left: parent.left
        anchors.right: parent.right
        height: 2
        color: Theme.accent
        visible: active
    }

    property bool active: false
    property bool hovered: false

    Text {
        id: label
        anchors.centerIn: parent
        text: "main.rs"
        font.family: Theme.fontUi
        font.pixelSize: Theme.sizeBody
        font.weight: active ? Theme.weightMedium : Theme.weightRegular
        color: active ? Theme.textPrimary : Theme.textSecondary
    }
}
```

### 9.8 Terminal（终端区域）

```qml
Rectangle {
    color: Theme.bgCanvas   // 终端用最深色

    // 终端文字必须 Mono
    // 字号 13px，行高 1.5
    // 颜色映射用 ANSI 16 调色板（见 §2.3）
}
```

---

## 10. Do's and Don'ts

### ✅ Do

- 永远从 `Theme.*` 取颜色和尺寸，**不要硬编码任何颜色或像素值**
- Inter Variable 必须开启 `cv01, ss03` OpenType features
- 默认 UI 文字 13px，IDE 是密集界面
- 主题切换必须有 200ms 颜色过渡（用 `Behavior on color`）
- 浮层、菜单、tooltip 必须有 inset top highlight 阴影（深色主题）
- 选中状态用 `accentMuted` 而非纯色填充
- 任何机器可读内容（IP、端口、命令、路径）用 Mono
- 跟随系统主题（监听 `QStyleHints::colorSchemeChanged`）

### ❌ Don't

- ❌ 不要用纯白 `#ffffff` 作为文字色（用 `#e8eaed`）
- ❌ 不要用纯黑 `#000000` 作为背景（用 `#0e0f11`）
- ❌ 不要用实色边框（如 `#2a2d31`）—— 永远半透明白
- ❌ 不要超过 590 字重，**禁止 bold (700+)**
- ❌ 不要在 13px 文字上用负字距
- ❌ 不要引入第二个强调色（红色 / 绿色 / 紫色装饰）
- ❌ 不要做 8px 大圆角的按钮 —— IDE 风格是 4px
- ❌ 不要用瞬间切换主题（必须带 200ms 过渡）
- ❌ 不要在 Pier-X 内部混入第三种字体
- ❌ 不要在终端区域使用 UI 字体
- ❌ 不要在边框 / 卡片上叠加多个不同强调色
- ❌ 不要用单层平面阴影 —— popover 必须多层 + inset

---

## 11. 应用清单（每次写 QML 时检查）

写完一个组件，对照下面列表过一遍：

- [ ] 所有颜色从 `Theme.*` 取
- [ ] 所有间距用 `Theme.sp*` 而不是裸数字
- [ ] 字体设了 `Theme.fontUi` 或 `Theme.fontMono`
- [ ] 字重用 `Theme.weightMedium` (510) 而非 500
- [ ] 边框用 `Theme.borderSubtle/Default/Strong`，不是实色
- [ ] 圆角用 `Theme.radiusSm` (4px)，不是 8px+
- [ ] hover / focus / 主题切换有 `Behavior on color` 过渡
- [ ] 阴影（如有）是多层 + inset top highlight
- [ ] 文字内容如果是机器可读 → Mono 字体
- [ ] 没有引入新的强调色
- [ ] 浅色 + 深色主题都测试过

---

## 12. 参考来源

本设计系统综合自以下 DESIGN.md（已提取到 `extracted/` 目录）：

| 来源 | 借鉴的部分 |
|---|---|
| [Linear](./extracted/linear.md) | luminance stacking、Inter 510 权重、半透明白边框、按钮 transparency 哲学 |
| [Warp](./extracted/warp.md) | 终端字体处理、克制的色板、editorial 节奏 |
| [Raycast](./extracted/raycast.md) | macOS 原生多层阴影 + inset highlight、popover 质感 |
| [Cursor](./extracted/cursor.md) | OpenType features 的运用、字距渐变 |
| JetBrains Darcula | 颜色饱和度、ANSI 调色板、密度感 |

**深度查阅**：原始的 9 节 DESIGN.md 格式（Visual Theme / Color / Typography / Components / Layout / Depth / Do's & Don'ts / Responsive / Agent Prompt Guide）保留在 `extracted/` 下，需要 component-level 灵感时直接查阅。

完整的原始仓库（58 个品牌的 DESIGN.md，包含 preview HTML）保留在 `reference/` 下作为只读资料。

---

**这套规范是 Pier-X 的视觉宪法。任何违反需要在 PR 描述中明确解释原因。**
