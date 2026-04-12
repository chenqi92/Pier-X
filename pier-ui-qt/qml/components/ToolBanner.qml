import QtQuick
import QtQuick.Layouts
import Pier

Rectangle {
    id: root

    property string text: ""
    property string tone: "neutral" // neutral | info | success | warning | error
    default property alias actions: actionRow.data

    readonly property color fillColor: {
        switch (root.tone) {
        case "info":
            return Theme.accentSubtle
        case "success":
            return Qt.rgba(Theme.statusSuccess.r, Theme.statusSuccess.g, Theme.statusSuccess.b, 0.10)
        case "warning":
            return Qt.rgba(Theme.statusWarning.r, Theme.statusWarning.g, Theme.statusWarning.b, 0.10)
        case "error":
            return Qt.rgba(Theme.statusError.r, Theme.statusError.g, Theme.statusError.b, 0.10)
        default:
            return Theme.bgSurface
        }
    }

    readonly property color strokeColor: {
        switch (root.tone) {
        case "info":
            return Theme.borderFocus
        case "success":
            return Theme.statusSuccess
        case "warning":
            return Theme.statusWarning
        case "error":
            return Theme.statusError
        default:
            return Theme.borderSubtle
        }
    }

    readonly property color textColor: {
        switch (root.tone) {
        case "info":
            return Theme.accent
        case "success":
            return Theme.statusSuccess
        case "warning":
            return Theme.statusWarning
        case "error":
            return Theme.statusError
        default:
            return Theme.textSecondary
        }
    }

    visible: root.text.length > 0
    implicitHeight: visible ? Math.max(28, contentRow.implicitHeight + Theme.sp2 * 2) : 0
    color: root.fillColor
    border.color: root.strokeColor
    border.width: 1
    radius: Theme.radiusSm
    clip: true

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
    Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

    RowLayout {
        id: contentRow
        anchors.fill: parent
        anchors.leftMargin: Theme.sp2
        anchors.rightMargin: Theme.sp2
        anchors.topMargin: Theme.sp1_5
        anchors.bottomMargin: Theme.sp1_5
        spacing: Theme.sp2

        Rectangle {
            width: 6
            height: 6
            radius: 3
            color: root.strokeColor
            Layout.alignment: Qt.AlignVCenter
        }

        Text {
            Layout.fillWidth: true
            text: root.text
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeCaption
            font.weight: Theme.weightMedium
            color: root.textColor
            elide: Text.ElideRight
            wrapMode: Text.NoWrap
            verticalAlignment: Text.AlignVCenter
        }

        RowLayout {
            id: actionRow
            spacing: Theme.sp1
        }
    }
}
