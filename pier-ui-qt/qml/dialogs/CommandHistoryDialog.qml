import QtQuick
import QtQuick.Controls
import QtQuick.Effects
import QtQuick.Layouts
import Pier
import "../components"

Item {
    id: root

    property bool opened: false
    property var onCommandSelected: null
    property var allHistory: []
    property var filteredHistory: []

    visible: opened
    anchors.fill: parent
    z: 9450

    function openDialog() {
        root.opened = true
        allHistory = PierCore.localHistory()
        filteredHistory = allHistory
        inputField.text = ""
        Qt.callLater(function() {
            inputField.forceActiveFocus()
            listView.currentIndex = filteredHistory.length > 0 ? 0 : -1
        })
    }

    function closeDialog() {
        root.opened = false
    }

    function open() {
        openDialog()
    }

    function close() {
        closeDialog()
    }

    function acceptCurrent() {
        if (listView.currentIndex < 0 || listView.currentIndex >= filteredHistory.length)
            return
        if (root.onCommandSelected)
            root.onCommandSelected(filteredHistory[listView.currentIndex])
        closeDialog()
    }

    ModalDialogShell {
        open: root.opened
        dialogWidth: 620
        dialogHeight: 420
        edgePadding: Theme.sp8 * 2
        title: qsTr("Command History")
        subtitle: qsTr("Search local command history and send the selected command to the active terminal.")
        bodyPadding: 0
        onRequestClose: root.closeDialog()

        body: ColumnLayout {
            anchors.fill: parent
            spacing: 0

            ToolPanelSurface {
                Layout.fillWidth: true
                padding: Theme.sp2
                implicitHeight: searchHeader.implicitHeight + Theme.sp2 * 2
                radius: 0

                RowLayout {
                    id: searchHeader
                    anchors.fill: parent
                    spacing: Theme.sp2

                    Image {
                        source: "qrc:/qt/qml/Pier/resources/icons/lucide/search.svg"
                        sourceSize: Qt.size(16, 16)
                        Layout.alignment: Qt.AlignVCenter
                        layer.enabled: true
                        layer.effect: MultiEffect {
                            colorizationColor: Theme.textSecondary
                            colorization: 1.0
                        }
                    }

                    PierSearchField {
                        id: inputField
                        Layout.fillWidth: true
                        placeholder: qsTr("Search command history...")
                        clearable: true

                        onTextChanged: {
                            const term = text.toLowerCase()
                            if (term === "") {
                                filteredHistory = allHistory
                            } else {
                                const filtered = []
                                for (let i = 0; i < allHistory.length; ++i) {
                                    if (allHistory[i].toLowerCase().indexOf(term) >= 0)
                                        filtered.push(allHistory[i])
                                }
                                filteredHistory = filtered
                            }
                            listView.currentIndex = filteredHistory.length > 0 ? 0 : -1
                            if (filteredHistory.length > 0)
                                listView.positionViewAtIndex(0, ListView.Beginning)
                        }

                        input.Keys.onPressed: (event) => {
                            if (event.key === Qt.Key_Down) {
                                if (listView.currentIndex < filteredHistory.length - 1)
                                    listView.currentIndex++
                                event.accepted = true
                            } else if (event.key === Qt.Key_Up) {
                                if (listView.currentIndex > 0)
                                    listView.currentIndex--
                                event.accepted = true
                            } else if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
                                root.acceptCurrent()
                                event.accepted = true
                            }
                        }
                    }
                }
            }

            ToolPanelSurface {
                Layout.fillWidth: true
                Layout.fillHeight: true
                inset: true
                padding: Theme.sp1_5

                ListView {
                    id: listView
                    anchors.fill: parent
                    clip: true
                    boundsBehavior: Flickable.StopAtBounds
                    model: root.filteredHistory
                    currentIndex: -1
                    spacing: Theme.sp0_5

                    delegate: Rectangle {
                        required property int index
                        required property string modelData

                        width: listView.width
                        implicitHeight: 32
                        radius: Theme.radiusSm
                        color: ListView.isCurrentItem ? Theme.bgSelected : (historyMouse.containsMouse ? Theme.bgHover : "transparent")

                        Behavior on color { ColorAnimation { duration: Theme.durFast } }

                        RowLayout {
                            anchors.fill: parent
                            anchors.leftMargin: Theme.sp3
                            anchors.rightMargin: Theme.sp3
                            spacing: Theme.sp2

                            Image {
                                source: "qrc:/qt/qml/Pier/resources/icons/lucide/terminal.svg"
                                sourceSize: Qt.size(14, 14)
                                Layout.alignment: Qt.AlignVCenter
                                layer.enabled: true
                                layer.effect: MultiEffect {
                                    colorizationColor: Theme.accent
                                    colorization: 1.0
                                }
                            }

                            Text {
                                text: modelData
                                Layout.fillWidth: true
                                Layout.alignment: Qt.AlignVCenter
                                font.family: Theme.fontMono
                                font.pixelSize: Theme.sizeCaption
                                color: Theme.textSecondary
                                elide: Text.ElideRight
                            }
                        }

                        MouseArea {
                            id: historyMouse
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onEntered: listView.currentIndex = index
                            onClicked: {
                                if (root.onCommandSelected)
                                    root.onCommandSelected(modelData)
                                root.closeDialog()
                            }
                        }
                    }

                    ToolEmptyState {
                        anchors.centerIn: parent
                        visible: listView.count === 0
                        icon: "search"
                        title: qsTr("No history match")
                        description: qsTr("Try another keyword to find a previously executed command.")
                    }

                    ScrollBar.vertical: PierScrollBar {
                        active: hovered || pressed
                        visible: listView.contentHeight > listView.height
                    }
                }
            }
        }

        footer: Item {
            implicitHeight: footerRow.implicitHeight

            RowLayout {
                id: footerRow
                width: parent.width
                spacing: Theme.sp2

                Text {
                    Layout.fillWidth: true
                    text: qsTr("%1 command(s)").arg(filteredHistory.length)
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeSmall
                    color: Theme.textTertiary
                }

                GhostButton {
                    text: qsTr("Close")
                    onClicked: root.closeDialog()
                }

                PrimaryButton {
                    text: qsTr("Insert")
                    enabled: listView.currentIndex >= 0
                    onClicked: root.acceptCurrent()
                }
            }
        }
    }
}
