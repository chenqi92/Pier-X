pragma Singleton
import QtQuick

// ─────────────────────────────────────────────────────────
// Pier-X Theme Singleton
//
// THE SINGLE SOURCE OF VISUAL TRUTH for all Pier-X QML.
// All colors, sizes, and motion values must come from here.
// See .claude/skills/pier-design-system/SKILL.md for the
// design rationale and the full token reference.
// ─────────────────────────────────────────────────────────
QtObject {
    id: theme

    // Theme switching --------------------------------------
    property bool dark: true

    // When true, follow the OS color scheme. Set to false the moment
    // the user manually toggles the theme — gives them full control.
    property bool followSystem: true

    // Called from C++ on startup and on QStyleHints::colorSchemeChanged.
    function setSystemScheme(systemDark) {
        if (followSystem) {
            dark = systemDark
        }
    }

    // Background — luminance stacking ----------------------
    readonly property color bgCanvas:    dark ? "#0e0f11" : "#fbfcfd"
    readonly property color bgPanel:     dark ? "#16181b" : "#f6f7f9"
    readonly property color bgSurface:   dark ? "#1c1e22" : "#ffffff"
    readonly property color bgElevated:  dark ? "#22252a" : "#ffffff"
    readonly property color bgHover:     dark ? Qt.rgba(1, 1, 1, 0.04) : Qt.rgba(0, 0, 0, 0.04)
    readonly property color bgActive:    dark ? Qt.rgba(1, 1, 1, 0.06) : Qt.rgba(0, 0, 0, 0.06)
    readonly property color bgSelected:  Qt.rgba(53 / 255, 116 / 255, 240 / 255, dark ? 0.16 : 0.10)

    // Text -------------------------------------------------
    readonly property color textPrimary:    dark ? "#e8eaed" : "#1e1f22"
    readonly property color textSecondary:  dark ? "#b4b8bf" : "#454850"
    readonly property color textTertiary:   dark ? "#868a91" : "#6c707e"
    readonly property color textDisabled:   dark ? "#5a5e66" : "#a7a9b0"
    readonly property color textInverse:    dark ? "#16181b" : "#ffffff"

    // Borders — always semi-transparent --------------------
    readonly property color borderSubtle:   dark ? Qt.rgba(1, 1, 1, 0.05) : Qt.rgba(0, 0, 0, 0.06)
    readonly property color borderDefault:  dark ? Qt.rgba(1, 1, 1, 0.09) : Qt.rgba(0, 0, 0, 0.10)
    readonly property color borderStrong:   dark ? Qt.rgba(1, 1, 1, 0.14) : Qt.rgba(0, 0, 0, 0.18)
    readonly property color borderFocus:    "#3574f0"

    // Single chromatic accent ------------------------------
    readonly property color accent:         "#3574f0"
    readonly property color accentHover:    "#4f8aff"
    readonly property color accentMuted:    Qt.rgba(53 / 255, 116 / 255, 240 / 255, 0.16)
    readonly property color accentSubtle:   Qt.rgba(53 / 255, 116 / 255, 240 / 255, 0.08)

    // Status colors ----------------------------------------
    readonly property color statusSuccess:  "#5fb865"
    readonly property color statusWarning:  "#f0a83a"
    readonly property color statusError:    "#fa6675"
    readonly property color statusInfo:     "#3574f0"

    // Typography -------------------------------------------
    readonly property string fontUi:    "Inter"
    readonly property string fontMono:  "JetBrains Mono"

    readonly property int sizeDisplay:  32
    readonly property int sizeH1:       24
    readonly property int sizeH2:       20
    readonly property int sizeH3:       16
    readonly property int sizeBodyLg:   14
    readonly property int sizeBody:     13
    readonly property int sizeCaption:  12
    readonly property int sizeSmall:    11

    readonly property int weightRegular:   400
    readonly property int weightMedium:    510
    readonly property int weightSemibold:  590

    // Spacing — 4px grid -----------------------------------
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

    // Border radius ----------------------------------------
    readonly property int radiusXs:    2
    readonly property int radiusSm:    4
    readonly property int radiusMd:    6
    readonly property int radiusLg:    8
    readonly property int radiusXl:    12
    readonly property int radiusPill:  9999

    // Motion -----------------------------------------------
    readonly property int durFast:     120
    readonly property int durNormal:   200
    readonly property int durSlow:     320
    readonly property int easingType:  Easing.OutCubic
}
