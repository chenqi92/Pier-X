import QtQuick
import QtQuick.Effects
import QtQuick.Controls.Basic
import QtQuick.Layouts
import Pier
import "../components"

// Global command history palette. Invoked by Ctrl+R.
// Uses local history loaded directly from C++.
Popup {
    id: root

    // Caller provides a function or signal to handle chosen command
    property var onCommandSelected: null

    width: Math.min(600, parent.width - Theme.sp6 * 2)
    height: Math.min(400, parent.height - Theme.sp6 * 2)
    x: Math.round((parent.width - width) / 2)
    y: Math.round(parent.height * 0.15)
    modal: true
    focus: true
    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside

    property var allHistory: []
    property var filteredHistory: []

    onOpened: {
        allHistory = PierCore.localHistory()
        filteredHistory = allHistory
        inputField.text = ""
        inputField.forceActiveFocus()
        listView.currentIndex = -1
    }

    background: Rectangle {
        color: Theme.bgElevated
        border.color: Theme.borderDefault
        border.width: 1
        radius: Theme.radiusLg

        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
        Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

        // Subtle drop shadow
        Rectangle {
            anchors.fill: parent
            z: -1
            color: "transparent"
            border.color: Qt.rgba(0,0,0,0.3)
            border.width: 1
            radius: Theme.radiusLg
            anchors.margins: -1
        }
    }

    contentItem: ColumnLayout {
        anchors.fill: parent
        spacing: 0

        // Search Header
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 48
            color: "transparent"
            border.color: Theme.borderSubtle
            border.width: 1
            // Hide all borders except bottom
            Rectangle {
                anchors.fill: parent
                color: Theme.bgElevated
                border.width: 0
                anchors.bottomMargin: 1
                z: -1
            }

            RowLayout {
                anchors.fill: parent
                anchors.margins: Theme.sp2
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

                TextField {
                    id: inputField
                    Layout.fillWidth: true
                    Layout.alignment: Qt.AlignVCenter
                    placeholderText: qsTr("Search command history...")
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeBody
                    color: Theme.textPrimary
                    background: null

                    onTextChanged: {
                        var term = text.toLowerCase()
                        if (term === "") {
                            filteredHistory = allHistory
                        } else {
                            var filtered = []
                            for (var i = 0; i < allHistory.length; i++) {
                                if (allHistory[i].toLowerCase().indexOf(term) >= 0) {
                                    filtered.push(allHistory[i])
                                }
                            }
                            filteredHistory = filtered
                        }
                        listView.currentIndex = filteredHistory.length > 0 ? 0 : -1
                        listView.positionViewAtIndex(0, ListView.Beginning)
                    }

                    Keys.onPressed: (event) => {
                        if (event.key === Qt.Key_Down) {
                            if (listView.currentIndex < filteredHistory.length - 1) {
                                listView.currentIndex++
                            }
                            event.accepted = true
                        } else if (event.key === Qt.Key_Up) {
                            if (listView.currentIndex > 0) {
                                listView.currentIndex--
                            }
                            event.accepted = true
                        } else if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
                            if (listView.currentIndex >= 0 && listView.currentIndex < filteredHistory.length) {
                                if (root.onCommandSelected) {
                                    root.onCommandSelected(filteredHistory[listView.currentIndex])
                                }
                                root.close()
                            }
                            event.accepted = true
                        }
                    }
                }
            }
        }

        // Results list
        ListView {
            id: listView
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: root.filteredHistory
            currentIndex: -1

            delegate: Rectangle {
                width: listView.width
                implicitHeight: 32
                color: ListView.isCurrentItem ? Theme.bgHover : "transparent"

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
                            colorizationColor: Theme.accentMuted
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
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onEntered: listView.currentIndex = index
                    onClicked: {
                        if (root.onCommandSelected) {
                            root.onCommandSelected(modelData)
                        }
                        root.close()
                    }
                }
            }

            ScrollBar.vertical: ScrollBar {
                active: true
            }
        }
    }
}
