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
    readonly property string fontUi: "Inter"
    readonly property string fontMono: "JetBrains Mono"

    readonly property int sizeDisplay: 32
    readonly property int sizeH1: 24
    readonly property int sizeH2: 20
    readonly property int sizeH3: 16
    readonly property int sizeBodyLg: 14
    readonly property int sizeBody: 13
    readonly property int sizeCaption: 12
    readonly property int sizeSmall: 11

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
    readonly property int topBarHeight: 42
    readonly property int tabBarHeight: 34
    readonly property int tabHeight: 30
    readonly property int statusBarHeight: 22
    readonly property int controlHeight: 30
    readonly property int fieldHeight: 34
    readonly property int compactRowHeight: 28
    readonly property int listRowHeight: 32
    readonly property int sidebarWidth: 244
    readonly property int rightSidebarWidth: 396
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
