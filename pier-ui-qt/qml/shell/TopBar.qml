import QtQuick
import QtQuick.Controls.Basic as Controls
import QtQuick.Layouts
import QtQuick.Window
import Pier
import "../components"

Rectangle {
    id: root

    implicitHeight: Theme.topBarHeight
    color: Theme.bgChrome
    property string contextTitle: qsTr("Workspace")
    readonly property var appWindow: root.Window.window

    // Keep the brand aligned with macOS traffic lights without leaving
    // an oversized dead zone before the Pier-X mark.
    readonly property int trafficLightReservedWidth: Qt.platform.os === "osx" ? 60 : 0
    readonly property int leadingContentMargin: trafficLightReservedWidth > 0
                                                ? trafficLightReservedWidth + Theme.sp2
                                                : Theme.sp3
    signal newSessionRequested
    signal settingsRequested

    // ── Menu action dispatch ──
    signal menuAction(string action)

    // Menu state driven by Main.qml
    property bool canCloseTab: false
    property bool sidebarVisible: true
    property bool isFullScreen: false
    property bool isMaximized: false

    // ── Internal menu-bar state ──
    property bool _menuBarActive: false
    property bool _switchingMenu: false

    function _closeAllMenus() {
        _switchingMenu = true
        fileMenu.close()
        editMenu.close()
        viewMenu.close()
        windowMenu.close()
        helpMenu.close()
        _switchingMenu = false
    }

    function _onMenuClosed() {
        if (_switchingMenu)
            return
        if (!fileMenu.visible && !editMenu.visible && !viewMenu.visible
                && !windowMenu.visible && !helpMenu.visible)
            _menuBarActive = false
    }

    function _shortcutText(action, macShortcut, defaultShortcut) {
        const item = appWindow ? appWindow.activeFocusItem : null
        const shortcutName = action + "Shortcut"
        if (item && item[shortcutName] !== undefined && item[shortcutName] !== "")
            return item[shortcutName]
        return Qt.platform.os === "osx" ? macShortcut : defaultShortcut
    }

    // ── Window helpers ──

    function toggleWindowZoom() {
        if (!appWindow)
            return

        if (appWindow.visibility === Window.Maximized)
            appWindow.showNormal()
        else
            appWindow.showMaximized()
    }

    function handleTitleBarDoubleClick() {
        if (!appWindow || appWindow.visibility === Window.FullScreen)
            return

        const action = PierWindowChrome.titleBarDoubleClickAction()
        if (action === PierWindowChrome.MinimizeAction) {
            appWindow.showMinimized()
            return
        }
        if (action === PierWindowChrome.NoAction)
            return

        toggleWindowZoom()
    }

    function beginSystemMove(localX, localY) {
        if (!appWindow || appWindow.visibility === Window.FullScreen)
            return

        if (appWindow.visibility === Window.Maximized) {
            const globalPoint = root.mapToGlobal(Qt.point(localX, localY))
            const horizontalRatio = Math.max(0.12, Math.min(0.88, localX / Math.max(1, root.width)))
            const restoredWidth = Math.max(1, appWindow.windowedWidth || appWindow.width)

            appWindow.showNormal()

            const restoredX = globalPoint.x - restoredWidth * horizontalRatio
            const restoredY = globalPoint.y - Math.min(localY, Theme.topBarHeight / 2)

            appWindow.x = Math.round(restoredX)
            appWindow.y = Math.max(0, Math.round(restoredY))
        }

        appWindow.startSystemMove()
    }

    // ── Inline components ──

    component MenuTriggerBtn: Item {
        id: trigBtn
        property string label
        property var targetMenu

        width: trigLabel.implicitWidth + Theme.sp2 * 2
        implicitHeight: root.implicitHeight

        Rectangle {
            anchors.fill: parent
            anchors.topMargin: 7
            anchors.bottomMargin: 7
            color: trigMA.containsMouse || (trigBtn.targetMenu && trigBtn.targetMenu.visible)
                   ? Theme.bgHover : "transparent"
            radius: Theme.radiusSm
            Behavior on color { ColorAnimation { duration: Theme.durFast } }
        }

        Text {
            id: trigLabel
            anchors.centerIn: parent
            text: trigBtn.label
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeSmall
            font.weight: Theme.weightMedium
            color: trigMA.containsMouse || (trigBtn.targetMenu && trigBtn.targetMenu.visible)
                   ? Theme.textPrimary : Theme.textSecondary
            Behavior on color { ColorAnimation { duration: Theme.durFast } }
        }

        MouseArea {
            id: trigMA
            anchors.fill: parent
            hoverEnabled: true
            onClicked: {
                root._menuBarActive = true
                trigBtn.targetMenu.popup(trigBtn, 0, trigBtn.height)
            }
            onContainsMouseChanged: {
                if (containsMouse && root._menuBarActive
                        && trigBtn.targetMenu && !trigBtn.targetMenu.visible) {
                    root._closeAllMenus()
                    trigBtn.targetMenu.popup(trigBtn, 0, trigBtn.height)
                }
            }
        }
    }

    component ThemedMenuItem: Controls.MenuItem {
        id: tmi
        property string shortcutText: ""

        leftPadding: Theme.sp3
        rightPadding: Theme.sp3
        topPadding: 0
        bottomPadding: 0

        indicator: Item {
            implicitWidth: tmi.checkable ? 20 : 0
            implicitHeight: 20
            x: Theme.sp1
            anchors.verticalCenter: parent.verticalCenter
            visible: tmi.checkable

            Text {
                anchors.centerIn: parent
                text: "✓"
                font.family: Theme.fontUi
                font.pixelSize: Theme.sizeBody
                color: Theme.accent
                visible: tmi.checked
            }
        }

        contentItem: RowLayout {
            spacing: Theme.sp6

            Text {
                text: tmi.text
                font.family: Theme.fontUi
                font.pixelSize: Theme.sizeBody
                font.weight: Theme.weightRegular
                color: tmi.enabled ? Theme.textPrimary : Theme.textDisabled
                Layout.fillWidth: true
            }

            Text {
                text: tmi.shortcutText
                font.family: Theme.fontUi
                font.pixelSize: Theme.sizeCaption
                color: Theme.textTertiary
                visible: tmi.shortcutText.length > 0
            }
        }

        background: Rectangle {
            implicitWidth: 240
            implicitHeight: Theme.compactRowHeight
            color: tmi.highlighted ? Theme.bgSelected : "transparent"
            radius: Theme.radiusSm
        }
    }

    component ThemedSeparator: Controls.MenuSeparator {
        topPadding: Theme.sp1
        bottomPadding: Theme.sp1
        leftPadding: Theme.sp3
        rightPadding: Theme.sp3
        contentItem: Rectangle {
            implicitHeight: 1
            color: Theme.borderSubtle
        }
        background: Item {}
    }

    // ── Drag region ──
    Item {
        id: dragRegion
        anchors.left: leftContent.right
        anchors.right: rightControls.left
        anchors.leftMargin: Theme.sp2
        anchors.rightMargin: Theme.sp2
        anchors.top: parent.top
        anchors.bottom: parent.bottom

        DragHandler {
            id: moveHandler
            target: null
            grabPermissions: PointerHandler.CanTakeOverFromAnything

            onActiveChanged: {
                if (active)
                    root.beginSystemMove(centroid.position.x, centroid.position.y)
            }
        }

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.LeftButton | Qt.RightButton
            hoverEnabled: true
            cursorShape: Qt.ArrowCursor
            onClicked: (mouse) => {
                if (mouse.button !== Qt.RightButton || !root.appWindow)
                    return

                const globalPoint = dragRegion.mapToGlobal(Qt.point(mouse.x, mouse.y))
                PierWindowChrome.showSystemMenu(root.appWindow, globalPoint.x, globalPoint.y)
            }
            onDoubleClicked: {
                mouse.accepted = true
                root.handleTitleBarDoubleClick()
            }
        }
    }

    // ── Left content: Brand + Menu triggers ──
    RowLayout {
        id: leftContent
        anchors.left: parent.left
        anchors.leftMargin: root.leadingContentMargin
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        spacing: 0

        Text {
            text: "Pier-X"
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeBody
            font.weight: Theme.weightSemibold
            color: Theme.textPrimary
        }

        Rectangle {
            width: 1
            height: 14
            color: Theme.borderSubtle
            Layout.leftMargin: Theme.sp2
            Layout.rightMargin: Theme.sp1
        }

        MenuTriggerBtn {
            label: qsTr("File")
            targetMenu: fileMenu
        }
        MenuTriggerBtn {
            label: qsTr("Edit")
            targetMenu: editMenu
        }
        MenuTriggerBtn {
            label: qsTr("View")
            targetMenu: viewMenu
        }
        MenuTriggerBtn {
            label: qsTr("Window")
            targetMenu: windowMenu
        }
        MenuTriggerBtn {
            label: qsTr("Help")
            targetMenu: helpMenu
        }
    }

    // ── Right controls ──
    RowLayout {
        id: rightControls
        anchors.right: parent.right
        anchors.rightMargin: Theme.sp3
        anchors.verticalCenter: parent.verticalCenter
        spacing: Theme.sp1

        IconButton {
            compact: true
            icon: "plus"
            tooltip: qsTr("New session")
            onClicked: root.newSessionRequested()
        }

        IconButton {
            compact: true
            icon: Theme.dark ? "sun" : "moon"
            tooltip: Theme.dark ? qsTr("Switch to light theme") : qsTr("Switch to dark theme")
            onClicked: {
                Theme.followSystem = false
                Theme.dark = !Theme.dark
            }
        }

        IconButton {
            compact: true
            icon: "settings"
            tooltip: qsTr("Settings")
            onClicked: root.settingsRequested()
        }
    }

    // ── Bottom border ──
    Rectangle {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        height: 1
        color: Theme.borderSubtle
    }

    // ── Menu popup background helper ──
    readonly property color _menuBg: Theme.bgElevated
    readonly property color _menuBorder: Theme.borderDefault

    // ── File menu ──
    Controls.Menu {
        id: fileMenu
        topPadding: Theme.sp1
        bottomPadding: Theme.sp1
        onClosed: root._onMenuClosed()

        background: Rectangle {
            implicitWidth: 260
            color: root._menuBg
            border.color: root._menuBorder
            border.width: 1
            radius: Theme.radiusMd
        }

        ThemedMenuItem {
            text: qsTr("New local terminal")
            shortcutText: Qt.platform.os === "osx" ? "⌘T" : "Ctrl+T"
            onTriggered: root.menuAction("newTerminal")
        }
        ThemedMenuItem {
            text: qsTr("New SSH connection…")
            shortcutText: Qt.platform.os === "osx" ? "⌘N" : "Ctrl+N"
            onTriggered: root.menuAction("newSsh")
        }
        ThemedMenuItem {
            text: qsTr("Open Markdown preview…")
            onTriggered: root.menuAction("openMarkdown")
        }
        ThemedSeparator {}
        ThemedMenuItem {
            text: qsTr("Close current tab")
            shortcutText: Qt.platform.os === "osx" ? "⌘W" : "Ctrl+W"
            enabled: root.canCloseTab
            onTriggered: root.menuAction("closeTab")
        }
        ThemedSeparator {}
        ThemedMenuItem {
            text: qsTr("Settings…")
            shortcutText: Qt.platform.os === "osx" ? "⌘," : "Ctrl+,"
            onTriggered: root.menuAction("settings")
        }
        ThemedSeparator {}
        ThemedMenuItem {
            text: qsTr("Exit")
            shortcutText: Qt.platform.os === "osx" ? "⌘Q" : "Ctrl+Q"
            onTriggered: Qt.quit()
        }
    }

    // ── Edit menu ──
    Controls.Menu {
        id: editMenu
        topPadding: Theme.sp1
        bottomPadding: Theme.sp1
        onClosed: root._onMenuClosed()

        background: Rectangle {
            implicitWidth: 260
            color: root._menuBg
            border.color: root._menuBorder
            border.width: 1
            radius: Theme.radiusMd
        }

        ThemedMenuItem {
            text: qsTr("Undo")
            shortcutText: Qt.platform.os === "osx" ? "⌘Z" : "Ctrl+Z"
            onTriggered: root.menuAction("undo")
        }
        ThemedMenuItem {
            text: qsTr("Redo")
            shortcutText: Qt.platform.os === "osx" ? "⇧⌘Z" : "Ctrl+Shift+Z"
            onTriggered: root.menuAction("redo")
        }
        ThemedSeparator {}
        ThemedMenuItem {
            text: qsTr("Cut")
            shortcutText: Qt.platform.os === "osx" ? "⌘X" : "Ctrl+X"
            onTriggered: root.menuAction("cut")
        }
        ThemedMenuItem {
            text: qsTr("Copy")
            shortcutText: root._shortcutText("copy", "⌘C", "Ctrl+C")
            onTriggered: root.menuAction("copy")
        }
        ThemedMenuItem {
            text: qsTr("Paste")
            shortcutText: root._shortcutText("paste", "⌘V", "Ctrl+V")
            onTriggered: root.menuAction("paste")
        }
        ThemedMenuItem {
            text: qsTr("Select All")
            shortcutText: root._shortcutText("selectAll", "⌘A", "Ctrl+A")
            onTriggered: root.menuAction("selectAll")
        }
    }

    // ── View menu ──
    Controls.Menu {
        id: viewMenu
        topPadding: Theme.sp1
        bottomPadding: Theme.sp1
        onClosed: root._onMenuClosed()

        background: Rectangle {
            implicitWidth: 260
            color: root._menuBg
            border.color: root._menuBorder
            border.width: 1
            radius: Theme.radiusMd
        }

        ThemedMenuItem {
            text: Theme.dark ? qsTr("Switch to light theme") : qsTr("Switch to dark theme")
            onTriggered: { Theme.followSystem = false; Theme.dark = !Theme.dark }
        }
        ThemedMenuItem {
            text: qsTr("Follow system theme")
            checkable: true
            checked: Theme.followSystem
            onTriggered: Theme.followSystem = checked
        }
        ThemedSeparator {}
        ThemedMenuItem {
            text: root.sidebarVisible ? qsTr("Hide right sidebar") : qsTr("Show right sidebar")
            shortcutText: Qt.platform.os === "osx" ? "⌃⇧G" : "Ctrl+Shift+G"
            onTriggered: root.menuAction("toggleSidebar")
        }
    }

    // ── Window menu ──
    Controls.Menu {
        id: windowMenu
        topPadding: Theme.sp1
        bottomPadding: Theme.sp1
        onClosed: root._onMenuClosed()

        background: Rectangle {
            implicitWidth: 260
            color: root._menuBg
            border.color: root._menuBorder
            border.width: 1
            radius: Theme.radiusMd
        }

        ThemedMenuItem {
            text: qsTr("Minimize")
            onTriggered: { if (root.appWindow) root.appWindow.showMinimized() }
        }
        ThemedMenuItem {
            text: root.isMaximized ? qsTr("Restore") : qsTr("Zoom")
            onTriggered: root.toggleWindowZoom()
        }
        ThemedMenuItem {
            text: root.isFullScreen ? qsTr("Exit Full Screen") : qsTr("Enter Full Screen")
            shortcutText: Qt.platform.os === "osx" ? "⌃⌘F" : "F11"
            onTriggered: root.menuAction("toggleFullScreen")
        }
    }

    // ── Help menu ──
    Controls.Menu {
        id: helpMenu
        topPadding: Theme.sp1
        bottomPadding: Theme.sp1
        onClosed: root._onMenuClosed()

        background: Rectangle {
            implicitWidth: 220
            color: root._menuBg
            border.color: root._menuBorder
            border.width: 1
            radius: Theme.radiusMd
        }

        ThemedMenuItem {
            text: qsTr("About Pier-X")
            onTriggered: root.menuAction("about")
        }
    }
}
