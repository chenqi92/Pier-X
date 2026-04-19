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
| 2 | **Single chromatic accent** | 全系统**只有一个**强调色：IntelliJ 蓝 `#3574F0`。状态色用克制的绿/黄/红。**禁止装饰性用色** |
| 3 | **Semi-transparent borders, never solid** | 边框总是 `rgba(white, 0.05–0.14)`（深色）或 `rgba(black, 0.06–0.18)`（浅色）。**禁止 dark-on-dark 实色边框** —— 那样看起来机械、过时 |
| 4 | **System font for UI, JetBrains Mono for code** | UI 字体**按平台取系统字体**（macOS SF Pro / Windows Segoe UI / 其它 Inter fallback），原生观感优先。**510 是签名权重**。Mono 用于终端、代码、路径、IP、端口 |
| 5 | **Density over spectacle** | 这是 IDE 级工具，不是营销页。**12px UI baseline、11px dense rows**、13–14px 仅用于大号标签与标题、4px 间距栅格、不要巨型标题、不要装饰渐变 |

> **检查任何 PR 时反问**：这一改动是否违反了上面五条之一？如果是，**不要合并**。

---

## 2. 颜色 Tokens

### 2.1 深色主题（默认）

```
背景层（luminance stacking — 越高的层 = 越亮的背景）
─────────────────────────────────────────────
bg.canvas          #0e0f11   主窗口最深背景
bg.panel           #16181b   停靠面板、侧边栏
bg.surface         #1c1e22   卡片、对话框、抬升表面
bg.elevated        #22252a   popover、菜单、tooltip
bg.hover           rgba(255,255,255,0.04)   hover 叠加（任何层之上）
bg.active          rgba(255,255,255,0.06)   按下/活跃
bg.selected        rgba(53,116,240,0.16)    选中（用 accent 着色）

文本
─────────────────────────────────────────────
text.primary       #e8eaed   主要文字（不是纯白！）
text.secondary     #b4b8bf   次要文字（描述、标签）
text.tertiary      #868a91   弱化文字（占位、metadata）
text.disabled      #5a5e66   禁用
text.inverse       #16181b   accent 按钮上的文字

边框（永远是半透明白！）
─────────────────────────────────────────────
border.subtle      rgba(255,255,255,0.05)   默认（输入框、卡片）
border.default     rgba(255,255,255,0.09)   常规（突出的边界）
border.strong      rgba(255,255,255,0.14)   强（重要分隔）
border.focus       #3574f0                   键盘焦点

强调色（**全系统唯一彩色**）
─────────────────────────────────────────────
accent.primary     #3574f0   IntelliJ 蓝（默认 / Linux fallback）
accent.hover       #4f8aff   hover
accent.muted       rgba(53,116,240,0.16)   蓝色背景填充
accent.subtle      rgba(53,116,240,0.08)   极淡蓝色背景

> **运行时覆盖**：在 macOS / Windows 上，`accent.primary`、`accent.hover`、`border.focus`、`bg.selected` 的色相由平台系统强调色决定
> （macOS `NSColor.controlAccentColor` / Windows `HKCU\SOFTWARE\Microsoft\Windows\DWM\AccentColor`），alpha 关系保持不变（muted=0.16、subtle=0.08、bg.selected 暗=0.12 亮=0.10）。
> 仅 Linux 等不可读平台回退到 `#3574f0`。**单一强调色原则**仍然成立：系统只允许一个色相作为 accent，运行时拿到什么就用什么。

状态色（克制使用，仅用于状态指示）
─────────────────────────────────────────────
status.success     #5fb865   运行中、成功
status.warning     #f0a83a   警告
status.error       #fa6675   错误（不是纯红 #ff0000！）
status.info        #3574f0   信息（与 accent 同色）
```

### 2.2 浅色主题（深色的镜像）

```
背景层
─────────────────────────────────────────────
bg.canvas          #fbfcfd
bg.panel           #f6f7f9
bg.surface         #ffffff
bg.elevated        #ffffff   配合阴影区分
bg.hover           rgba(0,0,0,0.04)
bg.active          rgba(0,0,0,0.06)
bg.selected        rgba(53,116,240,0.10)

文本
─────────────────────────────────────────────
text.primary       #1e1f22   不是纯黑！
text.secondary     #454850
text.tertiary      #6c707e
text.disabled      #a7a9b0
text.inverse       #ffffff

边框
─────────────────────────────────────────────
border.subtle      rgba(0,0,0,0.06)
border.default     rgba(0,0,0,0.10)
border.strong      rgba(0,0,0,0.18)
border.focus       #3574f0

强调色 / 状态色：与深色主题相同（保持品牌一致）
```

### 2.3 终端 ANSI 16 色调色板（两种主题通用）

终端区域使用专门的 ANSI 调色板，与 UI chrome 解耦。基于 JetBrains Darcula 调整：

```
        Normal      Bright
black   #1c1e22     #5a5e66
red     #fa6675     #ff8593
green   #5fb865     #7fcf85
yellow  #f0a83a     #ffc15c
blue    #3574f0     #5e92ff
magenta #c678dd     #d894ed
cyan    #56b6c2     #7fc8d1
white   #b4b8bf     #e8eaed
```

---

## 3. 字体 Tokens

### 3.1 字体家族

**UI 字体按平台映射**（复刻 Pier/SwiftUI 的原生观感）：

| Platform | UI Font | 说明 |
|---|---|---|
| macOS | `.SystemUIFont` (SF Pro) | 与原生 AppKit/SwiftUI 控件一致 |
| Windows | `Segoe UI` | Windows 11 的系统字体 |
| Linux / 其它 | `Inter Variable` | 捆绑 fallback，开启 `cv01, ss03` |

Mono 字体跨平台统一：

| | Mono Font | 说明 |
|---|---|---|
| 默认 | `JetBrains Mono` | 捆绑 (OFL)，IDE 标志性 |
| Win fallback | `Cascadia Code` | 系统自带 |

> **规则**：UI 字体优先使用系统字体（macOS/Windows），Inter 作为 Linux 与捆绑回退。Mono 永远捆绑 JetBrains Mono 不依赖系统。
> 当使用 Inter 时仍必须开启 OpenType `cv01, ss03`；系统字体不要套 Inter 的 feature 参数。

### 3.2 类型阶梯

**注意：IDE 工具的基础 UI 文字是 12px，不是营销页的 16px。** Pier (SwiftUI) 实际使用 11–12pt 混排，Pier-X 对齐这个密度。

| Role | Font | Size | Weight | Line Height | Tracking | 用途 |
|---|---|---|---|---|---|---|
| Display | 平台 UI | 28px | 510 | 1.20 | -0.5px | 欢迎页、空状态大字 |
| H1 | 平台 UI | 20px | 510 | 1.30 | -0.2px | 设置页主标题 |
| H2 | 平台 UI | 16px | 510 | 1.35 | -0.1px | 对话框标题 |
| H3 | 平台 UI | 14px | 510 | 1.40 | 0 | 卡片标题 |
| Body Large | 平台 UI | 13px | 400 | 1.50 | 0 | 主要阅读文本 |
| **Body** | **平台 UI** | **12px** | **400** | **1.45** | **0** | **默认 UI 文字（最常用）** |
| Body Emphasis | 平台 UI | 12px | 510 | 1.45 | 0 | 强调标签、菜单项 |
| UI Label | 平台 UI | 12px | 510 | 1.0 | 0 | 按钮、tab、工具栏 |
| Caption | 平台 UI | 11px | 510 | 1.40 | 0 | 状态栏、metadata |
| Small | 平台 UI | 10px | 510 | 1.40 | 0 | section 标签、tooltip、密集行 |
| Mono Code | JetBrains Mono | 12px | 400 | 1.50 | 0 | 终端、代码、SQL |
| Mono Small | JetBrains Mono | 11px | 400 | 1.45 | 0 | 内联代码、路径 |

### 3.3 字体规则

- **510 是签名权重**。所有 UI 标签、按钮、菜单项默认 510。Body 阅读文本用 400。强调用 590（最大）。**禁止使用 700 (bold)** —— Pier-X 的最重权重是 590。
- **大字号用紧凑字距**（-0.1 to -0.5px），≤13px 用 normal 字距。
- **只在使用 Inter 时开启 OpenType `cv01, ss03`**。macOS SF Pro / Windows Segoe UI 不套任何 feature 参数，走平台默认渲染（避免把系统字体"改造"成 Inter 风格，丢掉原生感）。
- **代码 / 路径 / IP / 端口 / 命令 必须用 Mono**。任何「机器可读」的内容都是 Mono 的范畴。
- **不要混入第三种 UI 字体家族**。每个平台只用一个系统字体 + 捆绑的 JetBrains Mono。

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

**常用组合**（对齐 Pier/SwiftUI 的紧凑节奏）：

- 按钮默认高度：**22px (Sm)** — 工具栏、内联、对话框次按钮
- 按钮放大高度：28px (Md) — 仅用于对话框主按钮 / CTA
- 按钮微缩高度：18px (Xs) — 行内删除 / 操作条等极密场景
- 按钮水平 padding：Sm = `spacing.2`(8px)；Md = `spacing.3`(12px)
- 输入框高度 / padding：22px / `spacing.2 spacing.2`
- 卡片内边距：`spacing.4`
- 工具栏行高：32px（精确，含统一标题栏时与交通灯同行）
- 页头高度：36px（过去 42 → 36，更接近 SwiftUI）
- 终端 tab bar 高度：32px（过去 36 → 32）
- 菜单项高度：24px（不是 28 —— 对齐 List Row 标准值）
- 导航条目（侧栏 NavItem）：22px
- 列表项高度：22px（紧凑，`LIST_ROW_H`）/ 28px（inset List，`LIST_ROW_INSET_H`）
- 表单行（FormRow，label+control）：24px（`FORM_ROW_H`）

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

**圆角上限**：全系统圆角 ≤ 8px（`radius.lg`）。`radius.xl` (12px) 已废弃，仅保留为历史 token，不要在新组件中使用。Pier (SwiftUI) 的实际使用区间是 2–8pt，Pier-X 对齐。

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

## QML Implementation Protocol

> **Status (2026-04)**: The Qt/QML shell has been retired. The active runtime is `pier-ui-gpui` (Rust + GPUI). This section is preserved as historical reference and pairs with the new **GPUI Implementation Protocol** at the end of this file. New UI work targets GPUI; consult that section first.

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
- UI 字体按平台取系统字体（macOS SF Pro / Windows Segoe UI / Linux Inter）
- 只在使用 Inter 时开启 `cv01, ss03`；系统字体不套任何 OpenType feature
- 默认 UI 文字 **12px**，IDE 是密集界面（对齐 Pier/SwiftUI 的 11–12pt 节奏）
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
- ❌ 不要用 `radius.xl` (12px) 或更大 —— 全系统圆角上限是 8px
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

---

## GPUI Implementation Protocol

> 适用于 `pier-ui-gpui/`（Rust + GPUI），与上方 QML Implementation Protocol 平行。所有 GPUI UI 代码必须遵守本节。

### A. Token 取值规则

GPUI 没有 QML 的 Theme Singleton，token 通过 `cx.global::<Theme>()` 全局注入。视图与组件统一调用 `crate::theme::theme(cx)` 拿到 `&Theme`。

| 视觉值 | QML 形式 | GPUI 形式 |
|---|---|---|
| 主背景 | `Theme.bgCanvas` | `theme(cx).color.bg_canvas` |
| 卡片背景 | `Theme.bgSurface` | `theme(cx).color.bg_surface` |
| 主文字 | `Theme.textPrimary` | `theme(cx).color.text_primary` |
| 强调蓝 | `Theme.accent` | `theme(cx).color.accent` |
| Subtle 边框 | `Theme.borderSubtle` | `theme(cx).color.border_subtle` |
| 间距 12px | `Theme.sp3` | `crate::theme::spacing::SP_3` |
| 圆角 4px | `Theme.radiusSm` | `crate::theme::radius::RADIUS_SM` |
| 字号 13px | `Theme.sizeBody` | `crate::theme::typography::SIZE_BODY` |
| 字重 510 | `Theme.weightMedium` | `crate::theme::typography::WEIGHT_MEDIUM` |
| UI 字体 | `Theme.fontUi` | `theme(cx).font_ui.clone()` |
| Mono 字体 | `Theme.fontMono` | `theme(cx).font_mono.clone()` |

颜色完整映射 SKILL.md §2.1（深色）/ §2.2（浅色）。深浅切换通过 `theme::toggle(cx)` + `cx.refresh_windows()`。

### B. 字体加载（平台系统字体 + 捆绑 Mono）

`main.rs` 启动时仍然 `add_fonts` **捆绑 Inter + JetBrains Mono**（后者永远用，前者作为非 macOS/Windows 平台的 UI fallback）：

```rust
cx.text_system().add_fonts(vec![
    Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Variable.ttf").as_slice().into()),
    Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf").as_slice().into()),
]).unwrap();
```

UI 字体选择在 `theme/mod.rs::default_ui_font_family()` 做 cfg 分支：

```rust
pub fn default_ui_font_family() -> &'static str {
    #[cfg(target_os = "macos")] { ".SystemUIFont" }
    #[cfg(target_os = "windows")] { "Segoe UI" }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))] { "Inter" }
}
```

OpenType features：只在 family == "Inter" 时启用 `cv01/ss03`；系统字体返回 `FontFeatures::default()`。

### C. 组件白名单

视图层（`src/views/`、`src/app/`）只允许使用以下组件 + GPUI 原生 layout 元素（`div`/`flex`/`gap`）。任何其他视觉组合必须先在 `src/components/` 封装。

```text
Button { Primary | Ghost | Icon }     ← 替代 PrimaryButton.qml / GhostButton.qml / IconButton.qml
Card                                   ← 替代 Card.qml
StatusPill { Success | Warning | Error | Info }   ← 替代 StatusPill.qml
SectionLabel                           ← 替代 SectionLabel.qml
IconBadge                              ← Welcome / dock 的品牌徽章
NavItem                                ← Workbench Sidebar 的导航条目（28px、active/hover 态）
Separator                              ← 替代 Separator.qml
text::display(s) / h1(s) / h2(s) / h3(s) / body(s) / caption(s) / mono(s)   ← 文本 helpers
```

待补充（PR5+）：`PopoverPanel`、`ModalDialogShell`、`PierTextField`、`SegmentedControl`、`ToggleSwitch`、`PierToolTip`。需要时按本文 §1-§4 的 token 表 + `forks/gpui-component-pier/` 内的同名实现作为参考。

### D. Forbidden 模式（PR review 拒绝标准）

下列模式在 `src/views/` 与 `src/app/` 下出现时**必须打回**：

```rust
// ❌ 直接颜色字面量
div().bg(rgb(0x10192c))
div().border_color(rgba(0xffffff_0d))

// ❌ 数字像素字面量（绕过 spacing token）
div().p(px(16.))
div().gap(px(8.))
div().h(px(28.))   // 例外：组件内部尺寸（Button 28px、StatusPill 18px、IconBadge 28px）允许，必须用 SKILL.md 章节注释来源

// ❌ 硬编码字体字符串
div().font_family("Inter")

// ❌ 视图内创建新视觉原子（应该新建组件）
div().bg(t.color.bg_surface).border_1().border_color(t.color.border_subtle).rounded(RADIUS_MD).p(SP_4)
```

### E. 组件实现约定

- **Struct + `#[derive(IntoElement)]` + `RenderOnce` 实现**，统一形态。
- 第一个参数永远是 `id: ElementId`（用稳定字符串字面量），交互组件必须）。
- 变体用 `enum`（`ButtonVariant::{Primary, Ghost, Icon}`），不为变体新建独立 struct。
- 链式 builder：`Button::primary("welcome-new-ssh", "New SSH").width(px(148.)).on_click(cb)`。
- hover/active 用 `InteractiveElement::hover(...)` / `.active(...)`，颜色取自 `t.color.bg_hover` / `t.color.bg_active`。
- focus 取 `t.color.border_focus`（`#3574F0` IntelliJ 蓝）。

### F. 与 CLAUDE.md 的关系

[`/CLAUDE.md`](../../../CLAUDE.md) 写的是 Rust 代码层规则（模块组织、命名、禁用 API），本节写的是视觉 token 与组件 contract。两者互补，不重复。

### G. 应用清单（每次写 GPUI 代码时检查）

- [ ] 视图层零 `rgb(`、`rgba(`、`px(<裸数字>)`、字体字符串
- [ ] 所有视觉值通过 `theme(cx).color.*` / `spacing::SP_*` / `radius::RADIUS_*` / `typography::SIZE_*`
- [ ] 新视觉原子已封装为 `src/components/<name>.rs` 的 struct
- [ ] 变体是 enum 而不是新 struct
- [ ] 交互组件有稳定 `ElementId`
- [ ] hover / active / focus 三态都用了对应 token 颜色
- [ ] dark + light 双主题都验证过（Cmd+Shift+L 切换）
- [ ] Inter / JetBrains Mono 通过 `add_fonts` 捆绑，未依赖系统字体
- [ ] 字重最高 590（WEIGHT_EMPHASIS），未出现 700/Bold
- [ ] UI 字体走平台系统字体（macOS SF Pro / Windows Segoe UI），Linux 回退 Inter；Mono 仍用捆绑的 JetBrains Mono
- [ ] Accent 色在 macOS/Windows 上跟随系统；Linux 回退 `#3574F0`
