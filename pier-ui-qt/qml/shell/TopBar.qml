import QtQuick
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

    function _openMenuFrom(triggerItem, popup) {
        if (!triggerItem || !popup)
            return
        _switchingMenu = true
        fileMenu.close()
        editMenu.close()
        viewMenu.close()
        windowMenu.close()
        helpMenu.close()
        const pos = triggerItem.mapToItem(root, 0, triggerItem.height + Theme.sp1)
        popup.x = Math.max(Theme.sp2,
                           Math.min(root.width - popup.width - Theme.sp2, pos.x))
        popup.y = Math.max(Theme.sp2, pos.y)
        popup.open()
        _switchingMenu = false
        _menuBarActive = true
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
                root._openMenuFrom(trigBtn, trigBtn.targetMenu)
            }
            onContainsMouseChanged: {
                if (containsMouse && root._menuBarActive
                        && trigBtn.targetMenu && !trigBtn.targetMenu.visible) {
                    root._openMenuFrom(trigBtn, trigBtn.targetMenu)
                }
            }
        }
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

    PopoverPanel {
        id: fileMenu
        width: 260
        onClosed: root._onMenuClosed()

        Column {
            width: fileMenu.width - fileMenu.padding * 2
            spacing: Theme.sp0_5

            PierMenuItem {
                width: parent.width
                text: qsTr("New local terminal")
                trailingText: Qt.platform.os === "osx" ? "⌘T" : "Ctrl+T"
                onClicked: {
                    fileMenu.close()
                    root.menuAction("newTerminal")
                }
            }
            PierMenuItem {
                width: parent.width
                text: qsTr("New SSH connection…")
                trailingText: Qt.platform.os === "osx" ? "⌘N" : "Ctrl+N"
                onClicked: {
                    fileMenu.close()
                    root.menuAction("newSsh")
                }
            }
            PierMenuItem {
                width: parent.width
                text: qsTr("Open Markdown preview…")
                onClicked: {
                    fileMenu.close()
                    root.menuAction("openMarkdown")
                }
            }
            Rectangle { width: parent.width; height: 1; color: Theme.borderSubtle }
            PierMenuItem {
                width: parent.width
                text: qsTr("Close current tab")
                trailingText: Qt.platform.os === "osx" ? "⌘W" : "Ctrl+W"
                enabled: root.canCloseTab
                onClicked: {
                    fileMenu.close()
                    root.menuAction("closeTab")
                }
            }
            Rectangle { width: parent.width; height: 1; color: Theme.borderSubtle }
            PierMenuItem {
                width: parent.width
                text: qsTr("Settings…")
                trailingText: Qt.platform.os === "osx" ? "⌘," : "Ctrl+,"
                onClicked: {
                    fileMenu.close()
                    root.menuAction("settings")
                }
            }
            Rectangle { width: parent.width; height: 1; color: Theme.borderSubtle }
            PierMenuItem {
                width: parent.width
                text: qsTr("Exit")
                trailingText: Qt.platform.os === "osx" ? "⌘Q" : "Ctrl+Q"
                onClicked: {
                    fileMenu.close()
                    Qt.quit()
                }
            }
        }
    }

    PopoverPanel {
        id: editMenu
        width: 260
        onClosed: root._onMenuClosed()

        Column {
            width: editMenu.width - editMenu.padding * 2
            spacing: Theme.sp0_5

            PierMenuItem {
                width: parent.width
                text: qsTr("Undo")
                trailingText: Qt.platform.os === "osx" ? "⌘Z" : "Ctrl+Z"
                onClicked: {
                    editMenu.close()
                    root.menuAction("undo")
                }
            }
            PierMenuItem {
                width: parent.width
                text: qsTr("Redo")
                trailingText: Qt.platform.os === "osx" ? "⇧⌘Z" : "Ctrl+Shift+Z"
                onClicked: {
                    editMenu.close()
                    root.menuAction("redo")
                }
            }
            Rectangle { width: parent.width; height: 1; color: Theme.borderSubtle }
            PierMenuItem {
                width: parent.width
                text: qsTr("Cut")
                trailingText: Qt.platform.os === "osx" ? "⌘X" : "Ctrl+X"
                onClicked: {
                    editMenu.close()
                    root.menuAction("cut")
                }
            }
            PierMenuItem {
                width: parent.width
                text: qsTr("Copy")
                trailingText: root._shortcutText("copy", "⌘C", "Ctrl+C")
                onClicked: {
                    editMenu.close()
                    root.menuAction("copy")
                }
            }
            PierMenuItem {
                width: parent.width
                text: qsTr("Paste")
                trailingText: root._shortcutText("paste", "⌘V", "Ctrl+V")
                onClicked: {
                    editMenu.close()
                    root.menuAction("paste")
                }
            }
            PierMenuItem {
                width: parent.width
                text: qsTr("Select All")
                trailingText: root._shortcutText("selectAll", "⌘A", "Ctrl+A")
                onClicked: {
                    editMenu.close()
                    root.menuAction("selectAll")
                }
            }
        }
    }

    PopoverPanel {
        id: viewMenu
        width: 260
        onClosed: root._onMenuClosed()

        Column {
            width: viewMenu.width - viewMenu.padding * 2
            spacing: Theme.sp0_5

            PierMenuItem {
                width: parent.width
                text: Theme.dark ? qsTr("Switch to light theme") : qsTr("Switch to dark theme")
                onClicked: {
                    viewMenu.close()
                    Theme.followSystem = false
                    Theme.dark = !Theme.dark
                }
            }
            PierMenuItem {
                width: parent.width
                text: qsTr("Follow system theme")
                checkable: true
                checked: Theme.followSystem
                active: Theme.followSystem
                onClicked: {
                    Theme.followSystem = !Theme.followSystem
                    viewMenu.close()
                }
            }
            Rectangle { width: parent.width; height: 1; color: Theme.borderSubtle }
            PierMenuItem {
                width: parent.width
                text: root.sidebarVisible ? qsTr("Hide right sidebar") : qsTr("Show right sidebar")
                trailingText: Qt.platform.os === "osx" ? "⌃⇧G" : "Ctrl+Shift+G"
                onClicked: {
                    viewMenu.close()
                    root.menuAction("toggleSidebar")
                }
            }
        }
    }

    PopoverPanel {
        id: windowMenu
        width: 260
        onClosed: root._onMenuClosed()

        Column {
            width: windowMenu.width - windowMenu.padding * 2
            spacing: Theme.sp0_5

            PierMenuItem {
                width: parent.width
                text: qsTr("Minimize")
                onClicked: {
                    windowMenu.close()
                    if (root.appWindow)
                        root.appWindow.showMinimized()
                }
            }
            PierMenuItem {
                width: parent.width
                text: root.isMaximized ? qsTr("Restore") : qsTr("Zoom")
                onClicked: {
                    windowMenu.close()
                    root.toggleWindowZoom()
                }
            }
            PierMenuItem {
                width: parent.width
                text: root.isFullScreen ? qsTr("Exit Full Screen") : qsTr("Enter Full Screen")
                trailingText: Qt.platform.os === "osx" ? "⌃⌘F" : "F11"
                onClicked: {
                    windowMenu.close()
                    root.menuAction("toggleFullScreen")
                }
            }
        }
    }

    PopoverPanel {
        id: helpMenu
        width: 220
        onClosed: root._onMenuClosed()

        Column {
            width: helpMenu.width - helpMenu.padding * 2
            spacing: Theme.sp0_5

            PierMenuItem {
                width: parent.width
                text: qsTr("About Pier-X")
                onClicked: {
                    helpMenu.close()
                    root.menuAction("about")
                }
            }
        }
    }
}
