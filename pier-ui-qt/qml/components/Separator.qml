import QtQuick
import Pier

// 1px line separator — horizontal by default, vertical when `vertical: true`.
Rectangle {
    property bool vertical: false

    implicitWidth: vertical ? 1 : 0
    implicitHeight: vertical ? 0 : 1
    color: Theme.borderSubtle

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
}
