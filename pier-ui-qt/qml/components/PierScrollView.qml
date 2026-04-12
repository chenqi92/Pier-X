import QtQuick
import QtQuick.Controls
import Pier

ScrollView {
    id: root

    clip: true

    ScrollBar.vertical: PierScrollBar {
        compact: true
        policy: ScrollBar.AsNeeded
    }

    ScrollBar.horizontal: PierScrollBar {
        compact: true
        orientation: Qt.Horizontal
        policy: ScrollBar.AsNeeded
    }
}
