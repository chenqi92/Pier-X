import QtQuick
import QtQuick.Effects
import QtQuick.Layouts
import Pier

// Floating command palette — Cmd+K / Ctrl+K to open.
// Spec: SKILL.md §6 (L4 popover with multi-layer shadow + inset highlight)
//
// `commands` is a JS array of { title, shortcut, action } objects supplied
// by the parent. The palette filters by case-insensitive substring match
// and exposes selectedIndex / arrow-key navigation / Enter to invoke.
Item {
    id: root

    property bool open: false
    property var commands: []

    visible: open
    z: 9000
    anchors.fill: parent

    function show() {
        // Reset animation state
        popup.scale = 0.96
        popup.opacity = 0
        open = true
        // Trigger entry animation
        popup.scale = 1.0
        popup.opacity = 1.0
        searchBox.text = ""
        searchBox.forceActiveFocus()
        rebuildFiltered()
    }

    function hide() {
        open = false
    }

    function toggle() {
        if (open) hide(); else show()
    }

    function rebuildFiltered() {
        filtered.clear()
        const q = searchBox.text.trim().toLowerCase()
        for (let i = 0; i < commands.length; ++i) {
            const c = commands[i]
            if (q.length === 0 || c.title.toLowerCase().indexOf(q) !== -1) {
                filtered.append({
                    title: c.title,
                    shortcut: c.shortcut || "",
                    originalIndex: i
                })
            }
        }
        if (resultsList.count > 0) {
            resultsList.currentIndex = 0
        }
    }

    function invokeCurrent() {
        if (resultsList.currentIndex < 0 || resultsList.currentIndex >= filtered.count)
            return
        const item = filtered.get(resultsList.currentIndex)
        const cmd = root.commands[item.originalIndex]
        hide()
        if (cmd && typeof cmd.action === "function") {
            cmd.action()
        }
    }

    ListModel { id: filtered }

    // Backdrop
    Rectangle {
        anchors.fill: parent
        color: "#000000"
        opacity: root.open ? 0.5 : 0.0

        Behavior on opacity { NumberAnimation { duration: Theme.durFast } }

        MouseArea {
            anchors.fill: parent
            enabled: root.open
            onClicked: root.hide()
        }
    }

    // Palette popup
    Rectangle {
        id: popup
        anchors.horizontalCenter: parent.horizontalCenter
        y: 120
        width: 560
        height: column.implicitHeight + Theme.sp3 * 2

        // Entry animation — scale-up + fade-in
        scale: 0.96
        opacity: 0
        Behavior on scale   { NumberAnimation { duration: Theme.durFast; easing.type: Easing.OutCubic } }
        Behavior on opacity { NumberAnimation { duration: Theme.durFast; easing.type: Easing.OutCubic } }
        transformOrigin: Item.Top

        color: Theme.bgElevated
        border.color: Theme.borderDefault
        border.width: 1
        radius: Theme.radiusMd

        Behavior on color        { ColorAnimation { duration: Theme.durNormal } }
        Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

        // Multi-layer shadow per SKILL.md L4 popover
        layer.enabled: true
        layer.effect: MultiEffect {
            shadowEnabled: true
            shadowColor: "#000000"
            shadowOpacity: 0.45
            shadowBlur: 1.0
            shadowVerticalOffset: 12
        }

        // Block clicks on the palette from reaching the backdrop.
        MouseArea {
            anchors.fill: parent
            onClicked: (mouse) => mouse.accepted = true
            onPressed: (mouse) => mouse.accepted = true
        }

        ColumnLayout {
            id: column
            anchors.fill: parent
            anchors.margins: Theme.sp3
            spacing: Theme.sp2

            PierSearchField {
                id: searchBox
                Layout.fillWidth: true
                placeholder: qsTr("Type a command…")
                clearable: true
                onTextChanged: root.rebuildFiltered()

                input.Keys.onDownPressed: {
                    if (resultsList.currentIndex < resultsList.count - 1)
                        resultsList.currentIndex++
                }
                input.Keys.onUpPressed: {
                    if (resultsList.currentIndex > 0)
                        resultsList.currentIndex--
                }
                input.Keys.onReturnPressed: root.invokeCurrent()
                input.Keys.onEnterPressed: root.invokeCurrent()
                input.Keys.onEscapePressed: root.hide()
            }

            Separator { Layout.fillWidth: true }

            ListView {
                id: resultsList
                Layout.fillWidth: true
                Layout.preferredHeight: Math.min(contentHeight, 320)
                model: filtered
                clip: true
                interactive: true
                currentIndex: 0

                delegate: Rectangle {
                    width: ListView.view.width
                    implicitHeight: 32
                    color: ListView.isCurrentItem
                         ? Theme.accentMuted
                         : itemArea.containsMouse
                             ? Theme.bgHover
                             : "transparent"
                    radius: Theme.radiusSm

                    Behavior on color { ColorAnimation { duration: Theme.durFast } }

                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: Theme.sp3
                        anchors.rightMargin: Theme.sp3
                        spacing: Theme.sp2

                        Text {
                            Layout.fillWidth: true
                            text: model.title
                            font.family: Theme.fontUi
                            font.pixelSize: Theme.sizeBody
                            font.weight: Theme.weightMedium
                            color: Theme.textPrimary
                            elide: Text.ElideRight
                        }

                        Text {
                            visible: model.shortcut.length > 0
                            text: model.shortcut
                            font.family: Theme.fontMono
                            font.pixelSize: Theme.sizeCaption
                            color: Theme.textTertiary
                        }
                    }

                    MouseArea {
                        id: itemArea
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            resultsList.currentIndex = index
                            root.invokeCurrent()
                        }
                    }
                }

                // Empty state
                Text {
                    anchors.centerIn: parent
                    visible: resultsList.count === 0
                    text: qsTr("No matching commands.")
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeBody
                    color: Theme.textTertiary
                }
            }
        }
    }
}
