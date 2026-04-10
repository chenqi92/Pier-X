# Pier-X 跨平台技术选型

> 目标：在 macOS 与 Windows 上重做 Pier 终端管理工具
> 约束：**非 Web 技术**、IDE 级美观（IntelliJ 风格）、启动快、内存低
> 复用：现有 `pier-core`（Rust）几乎可以全量保留

---

## 1. 现状评估：什么能复用，什么必须重做

| 模块 | 现状 | 跨平台情况 | 处置 |
|---|---|---|---|
| `pier-core/terminal` (vte) | Rust | ✅ 跨平台 | 直接复用 |
| `pier-core/terminal` (forkpty) | Unix only | ❌ Windows 无 | **需补 ConPTY 后端** |
| `pier-core/ssh` (russh + russh-sftp) | Rust | ✅ 跨平台 | 直接复用 |
| `pier-core/search` (ignore) | Rust | ✅ 跨平台 | 直接复用 |
| `pier-core/crypto` (ring) | Rust | ✅ 跨平台 | 直接复用 |
| `pier-core/git_graph` (git2) | Rust | ✅ 跨平台 | 直接复用 |
| `pier-core/ffi.rs` | C ABI | ✅ 跨平台 | 直接复用 |
| `PierApp/*` (SwiftUI + AppKit) | macOS only | ❌ | **必须重写** |
| Keychain（凭据存储） | macOS only | ❌ | 改用 `keyring` crate（自动选 DPAPI / Keychain） |

**结论**：核心引擎只需要补一个 ConPTY 后端（Windows 10 1809+ 自带），其他全部保留。重做的工作量集中在 UI 层与凭据存储。

---

## 2. 候选框架对比（已剔除所有 Web/Webview 方案）

| 方案 | 语言 | 渲染方式 | 二进制大小 | 冷启动 | 内存占用 | IDE 级美观 | 与 pier-core 集成 | 成熟度 |
|---|---|---|---|---|---|---|---|---|
| **Slint** | Rust + .slint DSL | GPU（Skia/软件） | 5–15 MB | <100 ms | 30–60 MB | ⭐⭐⭐⭐ | 🟢 同进程，0 FFI | ⭐⭐⭐ |
| **egui** | Rust | GPU（wgpu） | 5–10 MB | <50 ms | 20–40 MB | ⭐⭐ 偏「dev tool」 | 🟢 同进程，0 FFI | ⭐⭐⭐⭐ |
| **iced** | Rust | GPU（wgpu） | 8–15 MB | <100 ms | 30–60 MB | ⭐⭐⭐ | 🟢 同进程，0 FFI | ⭐⭐⭐ |
| **GPUI**（Zed 自研） | Rust | GPU（Metal/D3D11） | 10–20 MB | <50 ms | 40–80 MB | ⭐⭐⭐⭐⭐ | 🟢 同进程，0 FFI | ⭐⭐ 文档稀缺 |
| **Qt 6 / QML** | C++（或 Rust + cxx-qt） | GPU（RHI） | 30–60 MB | 200 ms | 80–150 MB | ⭐⭐⭐⭐⭐ | 🟡 C ABI 桥 | ⭐⭐⭐⭐⭐ |
| **Avalonia 11** | C# / .NET 8 AOT | GPU（Skia） | 30–80 MB | 100–300 ms | 60–120 MB | ⭐⭐⭐⭐⭐ | 🟡 P/Invoke | ⭐⭐⭐⭐ |
| **Compose Desktop** | Kotlin / JVM | Skia | 60–100 MB + JRE | 500 ms+ | 200 MB+ | ⭐⭐⭐⭐ | 🟡 JNI | ⭐⭐⭐⭐ |
| **Flutter Desktop** | Dart | Skia/Impeller | 30–50 MB | 200 ms | 80–150 MB | ⭐⭐⭐ 偏 Material | 🟡 FFI dart:ffi | ⭐⭐⭐⭐ |

> 已剔除：Tauri / Electron / Wails（均依赖 WebView）、Sciter（HTML/CSS 子集，仍是 Web 心智）。

---

## 3. 三个最佳选择

### 🥇 推荐 A：Rust + Slint（综合最优）

**为什么是它**：
- 纯 Rust，**与 `pier-core` 同进程零 FFI 开销**，可直接 `use pier_core::*`
- `.slint` 是声明式 DSL，热重载，UI 描述像 SwiftUI / QML
- 默认 Skia 后端，渲染质量接近 Qt
- 二进制 5–15 MB，启动 <100 ms，内存几十 MB —— 完全符合「快速、低占用」
- 提供 Fluent / Material / Cupertino 等内置 style，调成 IDE 风格不难
- 商业项目（如 Slint 自家工具、SixtyFPS 等）已在生产用

**短板**：
- 生态比 Qt 小，复杂控件（如代码编辑器、表格树）需要自己组合
- 终端文本渲染需要自己实现 glyph atlas（或集成 cosmic-text）

**项目结构（建议）**：
```
Pier-X/
├── pier-core/              # 软链接或 git submodule，引用现有 Rust 核心
│   └── (新增) terminal_win/  # ConPTY 后端
├── pier-x/                 # 新的 Rust UI crate
│   ├── Cargo.toml          # 依赖 slint = "1.x", pier-core = { path = "../pier-core" }
│   ├── build.rs            # slint-build
│   ├── src/
│   │   ├── main.rs
│   │   ├── view_models/
│   │   ├── components/
│   │   └── theme.rs
│   └── ui/
│       ├── main.slint
│       ├── terminal.slint
│       ├── sftp_panel.slint
│       └── theme/
│           ├── dark.slint   # IntelliJ Darcula 风格
│           └── light.slint
└── docs/
    └── TECH-STACK.md
```

---

### 🥈 推荐 B：Qt 6 + QML（IDE 美观度天花板）

**为什么是它**：
- KDevelop / Qt Creator / CLion / WebStorm 的底层渲染就是 Qt（JetBrains 用 Java 但部分原生层是 Qt）
- QML 声明式 UI，做出 IntelliJ 那种细腻的工具窗口、Tab、SplitView 最得心应手
- 控件库最完整：QTableView、QTreeView、QDockWidget、QSyntaxHighlighter 全都开箱即用
- 跨平台**最成熟**，HiDPI、IME、无障碍都打磨很久
- Rust 集成方案：`cxx-qt`（KDAB 出品）

**短板**：
- C++ 或 cxx-qt（学习成本最高）
- 二进制 30–60 MB，比 Slint 大几倍
- LGPL 协议需要注意动态链接（个人/开源完全没问题）

**适合场景**：你愿意接受更陡的学习曲线，换取**最接近 IntelliJ 的视觉与交互成熟度**。

---

### 🥉 推荐 C：C# + Avalonia 11（.NET AOT）

**为什么是它**：
- XAML 与 WPF/UWP 同源，**JetBrains Rider 的部分 UI 就是 Avalonia**（Rider 的 Settings、对话框等）
- 想做 IntelliJ 风格几乎是「天然契合」—— 社区有现成的 Fluent / Simple / IntelliJ 主题
- .NET 8 AOT 编译后启动可降到 100–200 ms，二进制 30–80 MB（含 runtime）
- 与 Rust 集成走 P/Invoke，pier-core 的 C ABI 直接能用
- 控件生态丰富（AvaloniaEdit 是开箱即用的代码编辑器，可以拿来做 SQL/日志面板）

**短板**：
- 仍是 GC 语言，内存比 Slint 稍高
- macOS 上的原生体验比 Qt/Slint 略弱（菜单、Dock 集成）

**适合场景**：你或团队熟悉 C#/.NET，想要**最快做出 IntelliJ 视觉**，对几十 MB 内存不敏感。

---

## 4. 决策矩阵

| 你的优先级 | 选 |
|---|---|
| **极致轻量 + 复用 Rust 核心** | 🥇 Slint |
| **IDE 美观度第一**，能接受 C++ | 🥈 Qt 6 / QML |
| **熟悉 C#/.NET，最快出活** | 🥉 Avalonia 11 |
| 想试前沿（GPU 直绘） | GPUI（不推荐第一次用，文档少） |
| 只是个内部工具，不在乎美观 | egui（最快上手，但视觉一般） |

---

## 5. 我的最终建议

> **首选 Slint**，备选 Avalonia。

理由：
1. 你已经在 Rust 核心上投入了大量工程（`pier-core` 八个测试、russh、vte、ring 全部就绪），**Slint 是唯一能让 UI 与核心同语言、同进程、同 cargo workspace 的方案** —— 这意味着 0 FFI 序列化开销、统一的错误类型、统一的 async runtime（tokio）。
2. 「快速、低占用」是你明确写在需求里的硬指标，Slint 在所有候选里最优。
3. IntelliJ 风格的核心是「灰阶 + 强调色 + 细线分割 + 等距字体」，这些都是 Slint 用 `.slint` DSL 几十行就能搭出来的。
4. 如果后续 Slint 的某个细节不够，**可以局部引入 cosmic-text 或 femtovg** 单独绘制终端区域，整体架构不动。

Avalonia 作为备选：如果开发过程中发现 Slint 的复杂控件（树表、停靠面板、可拖拽 Tab 组）成本过高，**可以换成 Avalonia 而几乎不动 pier-core**，因为 pier-core 暴露的是 C ABI。

---

## 6. 立即可以做的第一步（不绑定任何方案）

无论最终选哪个 UI 框架，下面这些都是值得**先做的、与 UI 解耦**的工作，全部在 `pier-core` 内：

1. **补 ConPTY 后端**：在 `pier-core/src/terminal/` 下新增 `pty_windows.rs`，用 `windows-rs` 调 `CreatePseudoConsole`，对外暴露与现有 `forkpty` 相同的 trait
2. **替换 Keychain**：把 `PierApp/Sources/Services/KeychainService.swift` 的能力下沉到 Rust，使用 `keyring = "3"` crate（自动适配 macOS Keychain / Windows Credential Manager / Linux Secret Service）
3. **抽离配置目录**：用 `directories` crate 替换硬编码的 `~/Library/Application Support/...`
4. **C ABI 表面回顾**：检查 `pier-core/src/ffi.rs` 当前导出的函数是否够用，给 UI 层留好接口

这一步做完，无论 UI 选 Slint 还是 Avalonia，都不会返工。

---

## 7. 风险与开放问题

| 风险 | 应对 |
|---|---|
| Slint 的表格/树控件不如 Qt 完善 | 提前在 PoC 阶段验证 SFTP 文件树、数据库表浏览器 |
| ConPTY 与 forkpty 行为差异（窗口大小、信号） | 抽象 PTY trait，两个后端独立测试 |
| Windows 上 SSH agent 协议（pageant）与 OpenSSH agent 不同 | 用 `russh-keys` 的 agent client，分别支持 |
| 字体渲染：Windows 上等距字体可用性 | 内置 JetBrains Mono / Cascadia Code 作为 fallback |
| HiDPI 缩放在 Windows 上的行为 | Slint 默认支持，需要在 manifest 声明 PerMonitorV2 DPI awareness |

---

## 8. 下一步（等你决定后我可以接着做）

- [ ] 确认 UI 框架（Slint / Qt / Avalonia / 其他）
- [ ] 在 `Pier-X/` 下初始化 cargo workspace 或对应工程结构
- [ ] 把 `pier-core` 通过 git submodule / path 依赖接入
- [ ] 写 PoC：一个能开 PTY、跑 `ls` 的最小窗口
- [ ] PoC：一个能 SSH 到远端、列目录的窗口

---

**给我你的偏好（或直接说"按你推荐的来"），我就开始动手搭骨架。**

---

## 9. 引入 RDP / VNC 后的修订

引入远程桌面协议后，选型逻辑会发生变化 —— 关键不在 UI 框架本身，而在**协议库生态**和**像素流的高频上传**。

### 9.1 推荐顺序的变化

| 之前（仅终端 + SSH + SFTP） | 加上 RDP / VNC |
|---|---|
| 🥇 Slint | 🥇 Slint（条件：先做渲染 PoC） |
| 🥈 Qt 6 | 🥈 Qt 6（升级为「零风险」备选） |
| 🥉 Avalonia | Avalonia（地位下降） |

### 9.2 库生态

**RDP**：
| 库 | 语言 | 评级 | 备注 |
|---|---|---|---|
| **IronRDP** | Rust | ⭐⭐⭐⭐⭐ 生产级 | Devolutions 商业网关在用，支持 NLA/CredSSP/RemoteFX/H.264，**自带 winit 渲染示例** |
| **FreeRDP** | C | ⭐⭐⭐⭐⭐ 事实标准 | Remmina/KRDC 在用，C ABI 任何语言都能绑 |
| rdp-rs | Rust | ⭐⭐ | 已被 IronRDP 取代 |

**VNC**：
| 库 | 语言 | 评级 |
|---|---|---|
| **libvncclient** (LibVNCServer) | C | 事实标准，C ABI |
| vnc-rs | Rust | 协议完整，生产化稍弱 |
| 自写 RFB | Rust | 协议比 RDP 简单一个数量级，可控 |

**关键事实**：IronRDP 是纯 Rust 且自带 winit 渲染示例。Slint 底层也用 winit。**Slint + IronRDP 是天然组合** —— 可以直接参考 `ironrdp-client` 的渲染管线。

### 9.3 像素流渲染：UI 框架对比

RDP/VNC 本质是「每秒 30–60 次往一块矩形区域上传 BGRA 缓冲」。1080p 单帧 ~8 MB，60 FPS 约 480 MB/s 纹理上传带宽 —— 现代 GPU 无压力，关键是 UI 框架是否提供**零拷贝路径**。

| 框架 | API | 评级 | 备注 |
|---|---|---|---|
| **Qt 6** | `QRhiWidget` + `QSGTexture` / `QQuickItem` | ⭐⭐⭐⭐⭐ | KRDC/Remmina 都这么干 |
| **Slint** | `SharedPixelBuffer` + `Image::from_rgba8()` | ⭐⭐⭐⭐ | 设计意图就是高频图像，需要 PoC 验证 |
| **Avalonia** | `WriteableBitmap` / `OpenGlControlBase` | ⭐⭐⭐⭐ | 社区有 `MarcusW.VncClient` Avalonia 实现 |
| egui/iced | `Context::load_texture` | ⭐⭐⭐ | 能用，但不是为持续高频流设计 |
| GPUI | 自定义 GPU 管线 | ⭐⭐ | 太底层 |

#### Slint 的两条渲染路径

1. **SharedPixelBuffer**（推荐起点）：把 `Vec<u8>` 包成 `slint::Image`，绑到 `Image` 元素上，每帧 `image = Image::from_rgba8(buffer)`，Slint 做纹理上传。
2. **slint::platform 自定义渲染**：完全接管某个区域的绘制，可直接调 wgpu/Metal/D3D11。**这是「逃生通道」** —— 如果方案 1 在某个分辨率/帧率下不够，方案 2 永远可行。

### 9.4 修订后的决策逻辑

如果 RDP/VNC 是**确定要做的核心功能**：

1. **先做 1 周 PoC**：Slint + IronRDP，跑通 Windows Server 1080p 窗口，测延迟与 CPU
2. **PoC 通过** → 整个项目继续 Slint，享受零 FFI 开销 + IronRDP 原生集成 + 5–15 MB 二进制
3. **PoC 不过**（4K 卡顿、纹理上传瓶颈、IME 冲突）→ 切换到 **Qt 6 + cxx-qt**，pier-core 不动，损失约 1–2 周 UI 重做

**Avalonia 为什么降级**：
- IronRDP 要 P/Invoke 包装（成本可控但繁琐）
- 或用 FreeRDP + C# 包装（更繁琐，.NET 的 RDP 库都不太活跃）
- 既没有 Slint 的 Rust 同源优势，也没有 Qt 的成熟先例 —— 「中间地带」反而不利

### 9.5 第三条路：混合架构

如果担心单一框架在 RDP 渲染上撞墙：

- **主框架用 Slint**（终端/SSH/SFTP/数据库面板等所有非视频内容）
- **RDP/VNC 视图开独立的 winit 子窗口**，里面直接用 `softbuffer` 或 `wgpu` + IronRDP，与主窗口通过 channel 通信
- 主窗口 Tab 上贴 placeholder，子窗口跟随移动

代价是窗口管理稍复杂，收益是 RDP 区域有完全独立、可控的渲染管线。

---

## 10. Qt 6 美观度深入：能不能做出 IntelliJ 级别？

### 10.1 第一件事：分清 Qt Widgets 和 Qt Quick (QML)

> 「Qt 丑」的刻板印象 99% 来自 Qt Widgets，不是 Qt Quick。

| 维度 | Qt Widgets | Qt Quick (QML) |
|---|---|---|
| 心智模型 | 基于 `QWidget` 类继承，命令式 | 声明式（像 SwiftUI / Compose / .slint） |
| 渲染 | 软件 + QPainter，部分 GPU | **GPU 优先（RHI: Metal / D3D11 / Vulkan / OpenGL）** |
| 默认风格 | 老派（像 2010 年的 Qt 4 应用） | 现代（Material / Universal / Fusion / Basic 可选） |
| 动画 | 弱（QPropertyAnimation） | 强（Behavior / States / Transitions / ParticleSystem） |
| 着色器 | 几乎没有 | `ShaderEffect` 直接写 GLSL/HLSL/MSL |
| 自定义视觉 | 困难（重写 paintEvent） | 简单（Rectangle + radius + gradient + DropShadow） |
| 适合做什么 | 老式工具栏 + 表格密集型工具 | 现代 IDE / 媒体应用 / 任何视觉要求高的 UI |

**结论：做 Pier-X 一定用 QML，不要用 Widgets**。这是 Qt 6 时代的官方推荐路径。

### 10.2 真实世界例子（按视觉精致度排序）

下面这些都是**纯 Qt 应用**，可以直接看到 Qt 能达到什么水平：

| 应用 | 框架 | 看点 |
|---|---|---|
| **Telegram Desktop** | Qt + 自定义样式 | 极其精致的动画、模糊背景、平滑滚动 —— 完全不像「Qt 应用」 |
| **Qt Design Studio** | QML + Qt Quick 3D | Qt 自家旗舰工具，深色 IDE 风，停靠面板、属性编辑器、3D 视图 |
| **Krita** | Qt Widgets（重度定制） | 数字绘画工具，工具栏复杂度堪比 Photoshop |
| **KDE Plasma 6** | QML | 整个桌面环境，模糊效果、图标动画、主题切换全部丝滑 |
| **OBS Studio** | Qt Widgets | 直播录屏，专业感强 |
| **Wireshark 4.x** | Qt Widgets | 包列表 + 详情 + 字节视图三栏，性能极佳 |
| **VLC** | Qt Widgets | 全平台，紧凑专业 |
| **Scribus / LMMS** | Qt | 专业出版/音乐制作 |

特别看 **Telegram Desktop** 和 **Qt Design Studio** —— 它们证明了 Qt 完全可以做出 macOS 原生级别的视觉。

### 10.3 复杂 UI 能力：能否做 IDE 级别？

**能。Qt 是所有候选里复杂 UI 能力最强的**。

| 复杂 UI 需求 | Qt 6 / QML 支持 |
|---|---|
| 可拖拽停靠面板（Dockable） | `QDockWidget`（Widgets）+ 第三方 **KDDockWidgets**（KDAB 出品，是当前 Qt 生态最强的停靠面板库，支持浮动、分组、持久化布局） |
| 可拖动重排 Tab | QML `TabBar` + `DragHandler`，或 KDDockWidgets |
| 树形 + 表格混合视图 | `TreeView` / `TableView` (QML) 或 `QTreeView` / `QTableView` (Widgets) |
| 代码编辑器（语法高亮、行号、补全） | **QScintilla**（Scintilla 的 Qt 绑定，VS Code 之前的事实标准）/ **KSyntaxHighlighting** / **Qt Creator 的 TextEditor 模块**（可剥离） |
| 终端控件 | **QTermWidget**（KDE Konsole 的剥离版） |
| 文件浏览器 | `QFileSystemModel` + `TreeView`，开箱即用 |
| 图表 | Qt Charts（商业版免费可用）/ Qt Quick Charts |
| 富文本/Markdown | `QTextDocument` 原生支持 Markdown |
| 拖放 | 内置 |
| 自定义形状 | `Shape` + `Path`（QML，矢量绘制） |
| 模糊背景 | `MultiEffect` (Qt 6.5+) / `FastBlur` |
| 阴影 | `MultiEffect` / `DropShadow` |
| 动画 | `Behavior on x { NumberAnimation { duration: 300; easing.type: Easing.OutCubic } }` —— 一行就是一个流畅过渡 |

**Qt Creator 自身**就是「Qt 能否做 IDE」的最佳证明。它有：项目树、多 Tab 编辑器、停靠的输出面板/调试面板/Git Blame、嵌入式终端、欢迎页、设置对话框 —— 与 IntelliJ 同级的复杂度。它的源码是公开的，**整个代码可以拿来抄交互模式**。

### 10.4 亮 / 暗主题切换

Qt 6.5+ 在主题这件事上做得**比 Slint 完整、比 Avalonia 略弱（Avalonia 的 FluentTheme 开箱更现代）、与 Windows/macOS 原生 API 集成最好**。

#### 三个层次

**层 1：跟随系统主题（自动）**

```cpp
// Qt 6.5+
auto scheme = QGuiApplication::styleHints()->colorScheme();
// Qt::ColorScheme::Light / Dark / Unknown

// 监听系统切换
connect(qApp->styleHints(), &QStyleHints::colorSchemeChanged,
        this, &MyApp::onColorSchemeChanged);
```

macOS / Windows 11 用户在系统设置里切换暗色，应用**实时跟随**，无需重启。

**层 2：QML 内置样式的主题切换**

```qml
import QtQuick.Controls.Material

ApplicationWindow {
    Material.theme: Material.Dark   // 或 Material.Light / Material.System
    Material.accent: Material.Blue
    Material.primary: Material.BlueGrey
}
```

或用 Universal（Fluent）风格：

```qml
import QtQuick.Controls.Universal
Universal.theme: Universal.Dark
Universal.accent: Universal.Cobalt
```

**层 3：完全自定义的 Theme Singleton（推荐做 IntelliJ 风格时用这个）**

```qml
// Theme.qml (Singleton)
pragma Singleton
import QtQuick

QtObject {
    property bool dark: true

    readonly property color bg:        dark ? "#1e1f22" : "#f7f8fa"   // IntelliJ Darcula bg
    readonly property color bgPanel:   dark ? "#2b2d30" : "#ffffff"
    readonly property color bgHover:   dark ? "#3a3d40" : "#e8eaed"
    readonly property color border:    dark ? "#393b40" : "#dfe1e5"
    readonly property color text:      dark ? "#dfe1e5" : "#1e1f22"
    readonly property color textDim:   dark ? "#868a91" : "#6c707e"
    readonly property color accent:    "#3574f0"   // IntelliJ blue
    readonly property color error:     dark ? "#fa6675" : "#e55765"
    readonly property color success:   dark ? "#5fb865" : "#369650"

    readonly property int radiusSm: 4
    readonly property int radiusMd: 6
    readonly property int spacingSm: 4
    readonly property int spacingMd: 8

    readonly property string fontMono: "JetBrains Mono"
    readonly property string fontUi:   "Inter"
}
```

然后在任何控件里：

```qml
Rectangle {
    color: Theme.bgPanel
    border.color: Theme.border
    radius: Theme.radiusSm

    Behavior on color { ColorAnimation { duration: 200 } }   // 主题切换有平滑过渡
}
```

切换主题：

```qml
Theme.dark = !Theme.dark    // 全局响应，所有绑定自动更新，带过渡动画
```

**Qt 是少数能做「主题切换平滑动画」的桌面框架** —— 因为 QML 的 Behavior 机制天然支持属性插值。Slint 也能，但代码量更大；Avalonia 能但需要主题字典切换；Electron 类方案要 CSS transition + 重排。

#### 与 macOS / Windows 的原生集成

- **macOS**：Qt 6 自动支持 NSAppearance，标题栏会跟着变（包括交通灯按钮的对比度）
- **Windows 11**：Qt 6.5+ 支持 Mica / Acrylic 背景（通过 `Window.flags` 加 `Qt.ExpandedClientAreaHint` + DWM 调用）
- **图标**：用 SVG + `ColorOverlay` 或 `MultiEffect`，一套图标自动适配两种主题

### 10.5 美观度上限 vs 默认水平 —— 必须坦白的事

| 维度 | Qt 6 给你免费的 | 想达到 IntelliJ 级别还需要做的 |
|---|---|---|
| 控件齐全度 | ⭐⭐⭐⭐⭐ 无需自造轮子 | — |
| 默认 Material 样式 | ⭐⭐⭐⭐ 现代感够 | 调色板要换成中性灰 + 单一强调色 |
| 默认 Universal (Fluent) 样式 | ⭐⭐⭐⭐ Win 11 风 | macOS 上略不协调 |
| 自定义主题 | ⭐⭐⭐⭐⭐ 上限极高 | 需要花 2–4 周打磨视觉规范 |
| 字体渲染 | ⭐⭐⭐⭐⭐ 子像素 + HiDPI | 选好字体（JetBrains Mono / Inter） |
| 间距 / 留白 | ⭐⭐⭐ 默认偏紧凑 | 自己定 spacing token |
| 图标 | ⭐⭐⭐ 内置基础 | 引入 Lucide / Phosphor SVG 图标库 |
| 动画 | ⭐⭐⭐⭐⭐ 写一行 Behavior 就是一个动画 | — |

**结论**：
- **「能否做出 IntelliJ 级别」答案是肯定的**。Qt 6 给你的是「无上限工具箱」，不是「免费的 IntelliJ 皮肤」。
- **「免费给的不是 IntelliJ 风格」**：默认更接近 KDE / Material / Fluent。想要 IntelliJ Darcula 那种「中性灰 + 蓝色强调」，必须自己定义 Theme Singleton。
- **「定制成本远低于 Slint」**：因为复杂控件全部齐全，你只需要调色板和样式，不需要自己写树表、停靠面板、代码编辑器。
- **「定制成本略高于 Avalonia」**：Avalonia 的 FluentTheme 默认就很接近 Fluent + 半个 IntelliJ。但 Avalonia 没有 KDDockWidgets / QScintilla 这些重型生态。

### 10.6 给 Pier-X 的具体建议（如果选 Qt 6）

技术栈：
- **Qt 6.7+** with **QML / Qt Quick Controls 2**
- **KDDockWidgets** 做主窗口的停靠面板系统（终端 Tab + SFTP 树 + 右侧工具面板）
- **QScintilla** 或 **Qt Creator TextEditor** 做 SQL / 日志 / Markdown 编辑器
- **QTermWidget** 做终端显示（或自己用 QQuickItem 包 vte）—— 需要评估
- **cxx-qt** (KDAB) 做 Rust ↔ Qt 绑定，pier-core 直接接入
- 自定义 **Theme Singleton** 实现 IntelliJ Darcula + 浅色双主题
- **JetBrains Mono** 等距字体（开源） + **Inter** UI 字体（开源）
- SVG 图标库 **Lucide** 或 **Phosphor**
- 主窗口标题栏：macOS 用原生交通灯，Windows 用自绘标题栏 + Mica 背景

预期效果：**完全可以做到与 IntelliJ IDEA 同级的视觉与交互**，但前期需要花 2–4 周搭设计系统（Theme + 间距 + 图标 + 基础组件库）。

---

## 11. 最终决策建议（含 RDP/VNC + 美观度考量）

把所有维度收敛到一句话：

| 你的偏好 | 选 |
|---|---|
| **想要一周内出 PoC，最看重轻量与 Rust 一体化** | Slint，先 PoC 验证 RDP 渲染 |
| **想要最高视觉上限 + 最完整控件生态 + 零远程桌面渲染风险** | Qt 6 + QML + cxx-qt |
| **不想碰 C++/Rust 之外的语言，想用 C# 快速搭 UI** | Avalonia（但 RDP 集成成本最高） |

我的个人推荐**保持不变**：**先 Slint PoC，撞墙后再切 Qt 6**。但如果你看完第 10 节觉得「Qt 6 的成熟度和生态我更安心」，**直接选 Qt 6 也是完全合理的决定** —— 它是这三个候选里**唯一能确定百分百做出 IntelliJ 级别视觉**的方案，代价是更陡的学习曲线和稍大的二进制。

---

**等你拍板，我就开始动手。**

---

## 12. 同进程混用 Slint + Qt 6 的可行性分析（与最终架构推荐）

> 提出问题：能否「Slint 做轻量部分 + Qt 6 做复杂 UI」？目标是健壮长期基座。

### 12.1 同进程混用：技术可行，工程不可取

| 障碍 | 细节 |
|---|---|
| **事件循环冲突** | Qt 主线程跑 `QApplication::exec()`，Slint 跑 `slint::run_event_loop()`，主线程只能有一个。寄生方案不在 Slint 的官方支持范围内 |
| **窗口/控件不能互相嵌入** | Qt 的 `QWindow` 与 Slint 的 `slint::Window` 之间没有原生嵌入 API。只能通过离屏渲染 + 纹理共享，丢失原生输入路由 / IME / 无障碍 |
| **构建系统复杂度爆炸** | 同时维护 cargo + cmake + cxx-qt + slint-build。CI 矩阵和新人上手时间翻倍 |
| **主题/字体/间距双重维护** | 同一套调色板和设计 token 必须在两个框架里各定义一遍并保持同步 |
| **键盘焦点 / 快捷键 / 拖放** | 跨工具包的焦点链是噩梦，每个交互细节都要手工桥接 |
| **二进制体积叠加** | Qt 6（30–60 MB）+ Slint（5–15 MB）= 50–80 MB，**Slint 的轻量优势完全消失** |
| **零生产先例** | 没有任何已知生产应用同进程混用 Qt + Slint。如果是好主意，早该有人做了 |

**结论**：**不要在同一个进程里混用 Slint 和 Qt**。

### 12.2 唯一合理的「混用」是多进程

```
┌─────────────────────────────────┐
│  主壳 (Qt 6 / QML)              │  ← 菜单、停靠面板、SSH/SFTP/数据库
└─────┬───────────────────────────┘
      │ IPC (本地 socket / shmem / Cap'n Proto)
      ├──────────────┬──────────────┐
      ▼              ▼              ▼
   ┌──────┐      ┌──────┐       ┌──────┐
   │RDP   │      │VNC   │       │Editor│
   │子进程│      │子进程│       │子进程│
   └──────┘      └──────┘       └──────┘
```

这是 VS Code / Chrome / Office 在用的模式。

**优点**：
- 子进程崩溃不影响主壳
- 每个子进程可以用最适合的技术（RDP 子进程用纯 winit + IronRDP，性能最优）
- UI 框架仍然只有一个（Qt 6），不存在同步问题

**代价**：IPC 协议设计、子窗口位置同步、生命周期管理都是工程量。**只在子模块复杂度真的需要时才引入**。

### 12.3 「健壮长期基座」的真正含义

需求被翻译成一条原则：

> **核心是资产，UI 是可替换的消耗品。**

10 年后回头看 Pier-X，真正有价值的是 `pier-core` 里的协议实现、状态机、加密、远程服务发现逻辑。UI 框架的兴衰不应威胁这些资产。

### 12.4 推荐的健壮架构

```
Pier-X/
├── pier-core/          ← Rust，零 UI 依赖，纯领域逻辑
│   ├── terminal/       ← PTY (forkpty + ConPTY)
│   ├── ssh/            ← russh, sftp
│   ├── rdp/            ← IronRDP wrapper
│   ├── vnc/            ← vnc client
│   ├── db/             ← MySQL / PG / Redis client
│   ├── git/            ← libgit2
│   └── api/            ← 暴露给 UI 的稳定 API（C ABI + Rust trait 双层）
│
├── pier-ipc/           ← 可选：如果未来走多进程，定义稳定 IPC schema
│
├── pier-ui-qt/         ← Qt 6 + QML 主壳（长期主力）
│   ├── theme/          ← Design system
│   ├── components/     ← 可复用 QML 组件
│   ├── views/          ← 业务视图
│   └── bridge/         ← cxx-qt，将 pier-core 暴露给 QML
│
└── (假设) pier-ui-X/   ← 永远不会构建，除非 Qt 在 10 年后真的过时
```

### 12.5 必须遵守的设计纪律

1. **pier-core 永远不依赖任何 UI 类型**。不知道 `QString`、`QObject`、`slint::Image` 是什么
2. **pier-core 的 API 用 C ABI + Rust trait 双层暴露**。C ABI 保证任何语言能接，Rust trait 保证类型安全
3. **业务逻辑（含 ViewModel 状态）尽量下沉到 pier-core**。UI 层只做「显示状态 + 转发事件」
4. **UI 层是薄的**。一个新功能 80% 代码在 pier-core，20% 在 UI 层
5. **UI 框架特性不能泄漏到 pier-core**。Qt 的信号/槽、QML 的 Q_PROPERTY 不能出现在核心 API 里，用 Rust channel 或 callback 替代
6. **任何依赖 UI 框架的代码必须在 `pier-ui-qt/` 之内**。从 import 路径就能看出来

### 12.6 这种架构带来的实际收益

- ✅ 假设 10 年后 Qt 6 真的撞墙，**只重写 UI 层**，pier-core 一行不动
- ✅ 想加 CLI 模式（headless Pier）？直接复用 pier-core
- ✅ 想做服务器端版本（pier-server 给团队用）？复用 pier-core，加一个 RPC/WebSocket 层
- ✅ 单元测试覆盖率高（核心是纯 Rust，无 UI 依赖天然好测试）
- ✅ 新人按层切入：写 Rust 的改 core，写 QML 的改 UI

### 12.7 最终选型修订：选 Qt 6（不是 Slint）

「健壮基座 + 长期迭代」的诉求把推荐推向 Qt 6：

| 维度 | Slint | Qt 6 |
|---|---|---|
| 项目年龄 | 2020 | **1995（30 年）** |
| LTS 承诺 | 商业版有 | **Qt 6 LTS 明确 5+ 年支持** |
| 复杂控件齐全度 | 中（很多需自造） | **极高（KDDockWidgets / QScintilla / QTermWidget）** |
| 长期 API 稳定性 | 仍在演进 | **极稳，30 年向后兼容文化** |
| 生产 IDE 先例 | 无 | **Qt Creator / KDevelop** |
| 招聘 / 社区 | 小众 | **巨大** |
| 文档 / 书籍 / SO | 一般 | **极丰富** |
| 美观上限 | 高 | **同样高（Telegram Desktop）** |
| 与 pier-core 集成 | 零 FFI | cxx-qt（成熟） |
| 二进制体积 | 5–15 MB | 30–60 MB（可接受） |

**Slint 的优势（轻量 + Rust 一体化 + 快速 PoC）依然存在，但与你的长期诉求不匹配。**

### 12.8 最终技术栈

- **Qt 6.7+ LTS** + **QML / Qt Quick Controls 2**
- **cxx-qt**（KDAB 出品）：Rust ↔ Qt 桥接，pier-core 直接接入
- **KDDockWidgets**：主窗口停靠面板系统（IntelliJ 风格的可拖动面板）
- **QScintilla**：代码 / SQL / 日志编辑器
- **QTermWidget**：终端显示（或自绘 QQuickItem 包 vte）
- 自定义 **Theme Singleton**：IntelliJ Darcula + 浅色双主题，含切换动画
- **JetBrains Mono** 等距字体 + **Inter** UI 字体（均开源）
- **Lucide** 或 **Phosphor** SVG 图标库
- 标题栏：macOS 原生交通灯，Windows 自绘 + Mica 半透明
- **Cap'n Proto**（保留）：未来如果要拆 RDP/VNC 子进程，作为 IPC 协议

### 12.9 实施路线图

**阶段 1：核心抽象层（2 周）**
- [ ] 为 pier-core 设计稳定的 C ABI + Rust trait 双层 API
- [ ] 补 Windows ConPTY 后端
- [ ] 替换 Keychain 为 `keyring` crate
- [ ] 用 `directories` crate 处理跨平台路径

**阶段 2：Qt 6 骨架（2 周）**
- [ ] 初始化 cmake + cxx-qt 工程，链接 pier-core
- [ ] 主窗口 + 标题栏 + 菜单
- [ ] Theme Singleton + 亮暗切换
- [ ] 集成 KDDockWidgets，搭出三栏停靠布局

**阶段 3：核心功能 Port（4–6 周）**
- [ ] 终端 Tab（QTermWidget 或自绘）
- [ ] SSH 连接管理器
- [ ] SFTP 文件浏览器
- [ ] Git 面板
- [ ] Markdown 预览

**阶段 4：远程上下文工具（4 周）**
- [ ] 远程服务发现
- [ ] SSH 隧道管理
- [ ] MySQL / Redis / PG 客户端
- [ ] Docker / 日志面板

**阶段 5：RDP / VNC（4 周）**
- [ ] IronRDP 集成（先在主进程内嵌入，性能不够再拆子进程）
- [ ] VNC 客户端
- [ ] 剪贴板 / 文件传输 / 多显示器

**阶段 6：打磨（持续）**
- [ ] 性能 profile
- [ ] 代码签名 + 公证 + 安装包
- [ ] 自动更新

---

**修订后的最终建议：Qt 6 + QML + cxx-qt，单一框架，按上述纪律设计 pier-core 与 UI 的边界。等你确认就开搭骨架。**
