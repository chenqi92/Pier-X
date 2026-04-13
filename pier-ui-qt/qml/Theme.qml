pragma Singleton
import QtQuick

// Pier-X Theme Singleton
// The single source of truth for color, density, typography, and motion.
QtObject {
    id: theme

    property bool dark: true
    property bool followSystem: true

    function setSystemScheme(systemDark) {
        if (followSystem)
            dark = systemDark
    }

    // ── Terminal theme ────────────────────────────────────
    // Index into `terminalThemes`. Persisted by the Settings
    // dialog via PierTerminalTheme singleton.
    property int terminalThemeIndex: 0

    // Each entry: { key, fg, bg, ansi[16] }
    readonly property var terminalThemes: [
        {
            key: "defaultDark",
            fg: "#e8eaed", bg: "#0f1115",
            ansi: ["#000000","#CD0000","#00CD00","#CDCD00","#3B78FF","#CD00CD","#00CDCD","#E5E5E5",
                   "#7F7F7F","#FF0000","#00FF00","#FFFF00","#5C5CFF","#FF00FF","#00FFFF","#FFFFFF"]
        },
        {
            key: "defaultLight",
            fg: "#1f2329", bg: "#fbfcfd",
            ansi: ["#000000","#CD0000","#00A000","#A07000","#0000EE","#CD00CD","#00A0A0","#666666",
                   "#555555","#FF0000","#00CD00","#CDCD00","#5C5CFF","#FF00FF","#00CDCD","#444444"]
        },
        {
            key: "solarizedDark",
            fg: "#839496", bg: "#002B36",
            ansi: ["#073642","#DC322F","#859900","#B58900","#268BD2","#D33682","#2AA198","#EEE8D5",
                   "#002B36","#CB4B16","#586E75","#657B83","#839496","#6C71C4","#93A1A1","#FDF6E3"]
        },
        {
            key: "dracula",
            fg: "#F8F8F2", bg: "#282A36",
            ansi: ["#21222C","#FF5555","#50FA7B","#F1FA8C","#BD93F9","#FF79C6","#8BE9FD","#F8F8F2",
                   "#6272A4","#FF6E6E","#69FF94","#FFFFA5","#D6ACFF","#FF92DF","#A4FFFF","#FFFFFF"]
        },
        {
            key: "monokai",
            fg: "#F8F8F2", bg: "#272822",
            ansi: ["#272822","#F92672","#A6E22E","#F4BF75","#66D9EF","#AE81FF","#A1EFE4","#F8F8F2",
                   "#75715E","#F92672","#A6E22E","#F4BF75","#66D9EF","#AE81FF","#A1EFE4","#F9F8F5"]
        },
        {
            key: "nord",
            fg: "#D8DEE9", bg: "#2E3440",
            ansi: ["#3B4252","#BF616A","#A3BE8C","#EBCB8B","#81A1C1","#B48EAD","#88C0D0","#E5E9F0",
                   "#4C566A","#BF616A","#A3BE8C","#EBCB8B","#81A1C1","#B48EAD","#8FBCBB","#ECEFF4"]
        }
    ]

    function terminalThemeName(themeRef) {
        var key = ""
        if (typeof themeRef === "number") {
            var theme = terminalThemes[themeRef]
            key = theme && theme.key ? theme.key : ""
        } else if (themeRef && themeRef.key) {
            key = themeRef.key
        }

        switch (key) {
        case "defaultDark":
            return qsTr("Default Dark")
        case "defaultLight":
            return qsTr("Default Light")
        case "solarizedDark":
            return qsTr("Solarized Dark")
        case "dracula":
            return qsTr("Dracula")
        case "monokai":
            return qsTr("Monokai")
        case "nord":
            return qsTr("Nord")
        default:
            return ""
        }
    }

    // Convenience — current palette resolved from index.
    readonly property var currentTerminalTheme: terminalThemes[terminalThemeIndex] || terminalThemes[0]

    // Backgrounds — luminance stacking
    readonly property color bgCanvas: dark ? "#0f1115" : "#fbfcfd"
    readonly property color bgChrome: dark ? "#14171c" : "#f5f6f8"
    readonly property color bgPanel: dark ? "#171a1f" : "#f4f6f9"
    readonly property color bgSurface: dark ? "#1d2127" : "#ffffff"
    readonly property color bgElevated: dark ? "#242931" : "#ffffff"
    readonly property color bgInset: dark ? "#11141a" : "#eef1f5"
    readonly property color bgHover: dark ? Qt.rgba(1, 1, 1, 0.05) : Qt.rgba(0, 0, 0, 0.05)
    readonly property color bgActive: dark ? Qt.rgba(1, 1, 1, 0.08) : Qt.rgba(0, 0, 0, 0.08)
    readonly property color bgSelected: Qt.rgba(53 / 255, 116 / 255, 240 / 255, dark ? 0.18 : 0.12)

    // Text
    readonly property color textPrimary: dark ? "#e8eaed" : "#1f2329"
    readonly property color textSecondary: dark ? "#b5b9c1" : "#454b55"
    readonly property color textTertiary: dark ? "#878c95" : "#727887"
    readonly property color textDisabled: dark ? "#5c6068" : "#a6aab4"
    readonly property color textInverse: dark ? "#16181b" : "#ffffff"

    // Borders
    readonly property color borderSubtle: dark ? Qt.rgba(1, 1, 1, 0.05) : Qt.rgba(0, 0, 0, 0.06)
    readonly property color borderDefault: dark ? Qt.rgba(1, 1, 1, 0.09) : Qt.rgba(0, 0, 0, 0.10)
    readonly property color borderStrong: dark ? Qt.rgba(1, 1, 1, 0.14) : Qt.rgba(0, 0, 0, 0.18)
    readonly property color borderFocus: "#3574f0"
    readonly property color splitHandleIdle: "transparent"
    readonly property color splitHandleHover: dark ? Qt.rgba(1, 1, 1, 0.028) : Qt.rgba(0, 0, 0, 0.024)
    readonly property color splitHandleActive: Qt.rgba(53 / 255, 116 / 255, 240 / 255, dark ? 0.12 : 0.08)
    readonly property color splitHandleLine: dark ? Qt.rgba(1, 1, 1, 0.08) : Qt.rgba(0, 0, 0, 0.09)
    readonly property color splitHandleLineActive: Qt.rgba(53 / 255, 116 / 255, 240 / 255, dark ? 0.52 : 0.40)

    // Accent
    readonly property color accent: "#3574f0"
    readonly property color accentHover: "#4f8aff"
    readonly property color accentMuted: Qt.rgba(53 / 255, 116 / 255, 240 / 255, dark ? 0.18 : 0.12)
    readonly property color accentSubtle: Qt.rgba(53 / 255, 116 / 255, 240 / 255, 0.08)

    // Status
    readonly property color statusSuccess: "#5fb865"
    readonly property color statusWarning: "#f0a83a"
    readonly property color statusError: "#fa6675"
    readonly property color statusInfo: "#3574f0"

    // Typography
    property string fontUi: "Inter"
    property string fontMono: "JetBrains Mono"
    property real uiScale: 1.0

    readonly property var uiFontFamilies: {
        if (Qt.platform.os === "windows")
            return ["Inter", "Segoe UI", "Microsoft YaHei UI"]
        if (Qt.platform.os === "osx")
            return ["Inter", "SF Pro Text", "Helvetica Neue"]
        return ["Inter", "Noto Sans", "DejaVu Sans", "Liberation Sans"]
    }

    readonly property var monoFontFamilies: {
        if (Qt.platform.os === "windows")
            return ["JetBrains Mono", "Cascadia Code", "Consolas", "Courier New"]
        if (Qt.platform.os === "osx")
            return ["JetBrains Mono", "SF Mono", "Menlo", "Monaco"]
        return ["JetBrains Mono", "Cascadia Mono", "Fira Code", "Source Code Pro", "DejaVu Sans Mono"]
    }

    // Terminal font size — adjustable via Settings slider.
    // Used by TerminalView's PierTerminalGrid binding.
    property int terminalFontSize: 13

    // Cursor style: 0 = Block, 1 = Beam, 2 = Underline
    property int cursorStyle: 0
    property bool cursorBlink: true

    // Scrollback buffer size (lines). Default matches Rust emulator.
    property int scrollbackLines: 10000

    // Bell behavior
    property bool visualBell: true
    property bool audioBell: false

    readonly property int sizeDisplay: Math.round(32 * uiScale)
    readonly property int sizeH1: Math.round(24 * uiScale)
    readonly property int sizeH2: Math.round(20 * uiScale)
    readonly property int sizeH3: Math.round(16 * uiScale)
    readonly property int sizeBodyLg: Math.round(14 * uiScale)
    readonly property int sizeBody: Math.round(13 * uiScale)
    readonly property int sizeCaption: Math.round(12 * uiScale)
    readonly property int sizeSmall: Math.round(11 * uiScale)

    readonly property int weightRegular: 400
    readonly property int weightMedium: 510
    readonly property int weightSemibold: 590

    // Spacing — 4px grid
    readonly property int sp0: 0
    readonly property int sp0_5: 2
    readonly property int sp1: 4
    readonly property int sp1_5: 6
    readonly property int sp2: 8
    readonly property int sp3: 12
    readonly property int sp4: 16
    readonly property int sp5: 20
    readonly property int sp6: 24
    readonly property int sp8: 32
    readonly property int sp10: 40
    readonly property int sp12: 48

    // Radius
    readonly property int radiusXs: 2
    readonly property int radiusSm: 4
    readonly property int radiusMd: 6
    readonly property int radiusLg: 8
    readonly property int radiusXl: 12
    readonly property int radiusPill: 9999

    // Shell density
    readonly property int windowMinWidth: 1080
    readonly property int windowMinHeight: 680
    readonly property int topBarHeight: 38
    readonly property int tabBarHeight: 36
    readonly property int tabHeight: 30
    readonly property int statusBarHeight: 24
    readonly property int controlHeight: 30
    readonly property int fieldHeight: 34
    readonly property int compactRowHeight: 28
    readonly property int listRowHeight: 32
    readonly property int sidebarWidth: 272
    readonly property int rightSidebarWidth: 400
    readonly property int toolRailWidth: 44
    readonly property int dialogHeaderHeight: 56
    readonly property int dialogFooterHeight: 60

    // Icon sizes
    readonly property int iconXs: 12
    readonly property int iconSm: 14
    readonly property int iconMd: 16
    readonly property int iconLg: 18

    // Motion
    readonly property int durFast: 120
    readonly property int durNormal: 200
    readonly property int durSlow: 320
    readonly property int easingType: Easing.OutCubic
}
