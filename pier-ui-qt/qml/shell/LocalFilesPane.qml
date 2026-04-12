import QtCore
import QtQuick
import QtQuick.Layouts
import Qt.labs.folderlistmodel
import Pier

// Local file browser for the left panel. Keeps the interaction
// intentionally lightweight: browse directories, surface the
// current path, and open markdown/text files in the preview tab.
Item {
    id: root

    signal markdownRequested(string filePath)
    signal openTerminalRequested()

    readonly property string homePath: StandardPaths.writableLocation(StandardPaths.HomeLocation)
    property string currentPath: homePath

    function _toFolderUrl(path) {
        if (!path || path.length === 0)
            return ""
        if (String(path).indexOf("file://") === 0)
            return String(path)
        if (Qt.platform.os === "windows")
            return "file:///" + String(path).replace(/\\/g, "/")
        return "file://" + path
    }

    function _normalizePath(path) {
        var value = String(path || "")
        if (value.indexOf("file://") === 0) {
            value = value.replace(/^file:\/\//, "")
        }
        return decodeURIComponent(value)
    }

    function _displayPath(path) {
        var normalized = _normalizePath(path)
        if (normalized.indexOf(root.homePath) === 0)
            return "~" + normalized.slice(root.homePath.length)
        return normalized
    }

    function _goUp() {
        var trimmed = root.currentPath.replace(/[\\\/]+$/, "")
        if (trimmed.length === 0 || trimmed === "/" || /^[A-Za-z]:$/.test(trimmed))
            return
        var slash = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"))
        if (slash <= 0) {
            if (Qt.platform.os === "windows" && trimmed.length >= 2)
                root.currentPath = trimmed.slice(0, 2)
            else
                root.currentPath = "/"
            return
        }
        root.currentPath = trimmed.slice(0, slash)
    }

    function _isPreviewable(path) {
        var lower = String(path || "").toLowerCase()
        return lower.endsWith(".md")
                || lower.endsWith(".markdown")
                || lower.endsWith(".mdx")
                || lower.endsWith(".txt")
                || lower.endsWith(".log")
    }

    FolderListModel {
        id: folderModel
        folder: root._toFolderUrl(root.currentPath)
        showDirs: true
        showFiles: true
        showHidden: false
        showDotAndDotDot: false
        showDirsFirst: true
        sortField: FolderListModel.Name
        sortReversed: false
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: Theme.sp2

        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.sp2

            SectionLabel {
                text: qsTr("Files")
                Layout.fillWidth: true
            }

            IconButton {
                icon: "folder"
                tooltip: qsTr("Home")
                onClicked: root.currentPath = root.homePath
            }

            IconButton {
                icon: "arrow-left"
                tooltip: qsTr("Up")
                enabled: root.currentPath.length > 1
                onClicked: root._goUp()
            }

            IconButton {
                icon: "terminal"
                tooltip: qsTr("Terminal")
                onClicked: root.openTerminalRequested()
            }
        }

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 28
            color: Theme.bgSurface
            border.color: Theme.borderSubtle
            border.width: 1
            radius: Theme.radiusSm

            Text {
                anchors.fill: parent
                anchors.leftMargin: Theme.sp2
                anchors.rightMargin: Theme.sp2
                verticalAlignment: Text.AlignVCenter
                text: root._displayPath(root.currentPath)
                font.family: Theme.fontMono
                font.pixelSize: Theme.sizeCaption
                color: Theme.textSecondary
                elide: Text.ElideMiddle
            }
        }

        Text {
            Layout.fillWidth: true
            text: qsTr("Browse local files and click a Markdown or text file to open a preview tab.")
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeSmall
            color: Theme.textTertiary
            wrapMode: Text.WordWrap
        }

        ListView {
            id: filesView
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            spacing: Theme.sp0_5
            model: folderModel

            delegate: Rectangle {
                id: fileRow

                required property string fileName
                required property string filePath
                required property bool fileIsDir
                required property string fileSuffix
                required property date fileModified

                width: ListView.view.width
                implicitHeight: 36
                radius: Theme.radiusSm
                color: fileMouse.containsMouse ? Theme.bgHover : "transparent"

                Behavior on color { ColorAnimation { duration: Theme.durFast } }

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: Theme.sp2
                    anchors.rightMargin: Theme.sp2
                    spacing: Theme.sp2

                    Image {
                        Layout.preferredWidth: 16
                        Layout.preferredHeight: 16
                        source: fileRow.fileIsDir 
                                ? "qrc:/qt/qml/Pier/resources/icons/lucide/folder.svg"
                                : "qrc:/qt/qml/Pier/resources/icons/lucide/file-text.svg"
                        sourceSize: Qt.size(16, 16)
                        layer.enabled: true
                        layer.effect: MultiEffect {
                            colorizationColor: fileRow.fileIsDir ? Theme.accent : Theme.textTertiary
                            colorization: 1.0
                        }
                    }

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 0

                        Text {
                            Layout.fillWidth: true
                            text: fileRow.fileName
                            font.family: fileRow.fileIsDir ? Theme.fontUi : Theme.fontMono
                            font.pixelSize: Theme.sizeBody
                            font.weight: fileRow.fileIsDir ? Theme.weightMedium : Theme.weightRegular
                            color: Theme.textPrimary
                            elide: Text.ElideRight
                        }

                        Text {
                            Layout.fillWidth: true
                            text: fileRow.fileIsDir
                                  ? root._displayPath(fileRow.filePath)
                                  : Qt.formatDateTime(fileRow.fileModified, "yyyy-MM-dd HH:mm")
                            font.family: Theme.fontUi
                            font.pixelSize: Theme.sizeSmall
                            color: Theme.textTertiary
                            elide: Text.ElideRight
                        }
                    }
                }

                MouseArea {
                    id: fileMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        if (fileRow.fileIsDir) {
                            root.currentPath = fileRow.filePath
                        } else if (root._isPreviewable(fileRow.filePath)) {
                            root.markdownRequested(fileRow.filePath)
                        }
                    }
                }
            }

            Rectangle {
                anchors.centerIn: parent
                width: parent.width
                color: "transparent"
                visible: folderModel.count === 0

                Column {
                    anchors.centerIn: parent
                    spacing: Theme.sp1

                    Text {
                        horizontalAlignment: Text.AlignHCenter
                        text: qsTr("This folder is empty.")
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeBody
                        color: Theme.textSecondary
                    }

                    Text {
                        horizontalAlignment: Text.AlignHCenter
                        text: qsTr("Choose another directory or open a local terminal.")
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeSmall
                        color: Theme.textTertiary
                    }
                }
            }
        }
    }
}
