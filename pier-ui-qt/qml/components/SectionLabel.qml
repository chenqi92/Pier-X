import QtQuick
import Pier

// Small uppercase section label — editorial categorization marker.
Text {
    font.family: Theme.fontUi
    font.pixelSize: Theme.sizeCaption
    font.weight: Theme.weightMedium
    font.capitalization: Font.AllUppercase
    font.letterSpacing: 0.6
    color: Theme.textTertiary

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
}
