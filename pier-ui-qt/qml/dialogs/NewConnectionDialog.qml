import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Effects
import QtQuick.Layouts
import Pier

// Modal dialog — collect SSH connection details and emit `saved` with the
// resulting object. Styled closer to the original Pier sheet: clear grouping,
// padded content, and direct auth-mode switching.
Item {
    id: root

    property bool open: false
    readonly property int pagePadding: 28
    signal saved(var connection)
    signal cancelled

    visible: open
    z: 9500
    anchors.fill: parent

    readonly property bool passwordAuth: authMode.currentIndex === 0
    readonly property bool keyAuth: authMode.currentIndex === 1
    readonly property bool agentAuth: authMode.currentIndex === 2

    function show() {
        nameField.text = ""
        hostField.text = ""
        portField.text = "22"
        userField.text = ""
        passwordField.text = ""
        keyPathField.text = ""
        passphraseField.text = ""
        authMode.currentIndex = 0
        dialog.scale = 0.96
        dialog.opacity = 0
        open = true
        dialog.scale = 1.0
        dialog.opacity = 1.0
        hostField.forceActiveFocus()
    }

    function hide() {
        open = false
    }

    function submit() {
        const displayName = nameField.text.trim().length > 0
                ? nameField.text.trim()
                : hostField.text.trim()
        const conn = {
            name: displayName,
            host: hostField.text.trim(),
            port: parseInt(portField.text, 10) || 22,
            username: userField.text.trim(),
            authKind: root.passwordAuth ? "password"
                    : root.keyAuth ? "private_key"
                    : "agent",
            password: passwordField.text,
            privateKeyPath: keyPathField.text.trim(),
            passphrase: passphraseField.text
        }
        root.saved(conn)
        root.hide()
    }

    Keys.onEscapePressed: {
        root.cancelled()
        root.hide()
    }

    Rectangle {
        anchors.fill: parent
        color: "#000000"
        opacity: root.open ? 0.5 : 0.0

        Behavior on opacity { NumberAnimation { duration: Theme.durFast } }

        MouseArea {
            anchors.fill: parent
            enabled: root.open
            onClicked: {
                root.cancelled()
                root.hide()
            }
        }
    }

    Rectangle {
        id: dialog
        anchors.centerIn: parent
        width: Math.min(688, parent.width - Theme.sp8 * 2)
        height: Math.min(720, parent.height - Theme.sp8 * 2)
        scale: 0.96
        opacity: 0
        transformOrigin: Item.Center
        color: Theme.bgElevated
        border.color: Theme.borderDefault
        border.width: 1
        radius: Theme.radiusLg

        Behavior on scale { NumberAnimation { duration: Theme.durNormal; easing.type: Theme.easingType } }
        Behavior on opacity { NumberAnimation { duration: Theme.durNormal; easing.type: Theme.easingType } }
        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
        Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

        layer.enabled: true
        layer.effect: MultiEffect {
            shadowEnabled: true
            shadowColor: "#000000"
            shadowOpacity: 0.42
            shadowBlur: 1.0
            shadowVerticalOffset: 16
        }

        MouseArea {
            anchors.fill: parent
            onClicked: (mouse) => mouse.accepted = true
            onPressed: (mouse) => mouse.accepted = true
        }

        ColumnLayout {
            anchors.fill: parent
            spacing: 0

            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: Theme.dialogHeaderHeight
                color: Theme.bgChrome
                radius: Theme.radiusLg

                Rectangle {
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    height: Theme.radiusLg
                    color: Theme.bgPanel
                }

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: Theme.sp5
                    anchors.rightMargin: Theme.sp3
                    spacing: Theme.sp3

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 0

                        Text {
                            text: qsTr("New SSH connection")
                            font.family: Theme.fontUi
                            font.pixelSize: Theme.sizeH3
                            font.weight: Theme.weightMedium
                            color: Theme.textPrimary
                        }

                        Text {
                            text: qsTr("Create a reusable host profile for SSH, SFTP, and service panels.")
                            font.family: Theme.fontUi
                            font.pixelSize: Theme.sizeSmall
                            color: Theme.textTertiary
                        }
                    }

                    IconButton {
                        icon: "x"
                        tooltip: qsTr("Close")
                        onClicked: {
                            root.cancelled()
                            root.hide()
                        }
                    }
                }

                Rectangle {
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    height: 1
                    color: Theme.borderSubtle
                }
            }

            ScrollView {
                id: formScroll
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                contentWidth: availableWidth

                ColumnLayout {
                    id: form
                    width: Math.max(0, formScroll.availableWidth - root.pagePadding * 2)
                    x: root.pagePadding
                    y: root.pagePadding
                    spacing: Theme.sp6

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: Theme.sp3

                        SectionLabel { text: qsTr("Connection") }

                        FieldGroup {
                            label: qsTr("Name")
                            description: qsTr("Optional display name shown in the sidebar.")

                            PierTextField {
                                id: nameField
                                Layout.fillWidth: true
                                placeholder: qsTr("My production server")
                            }
                        }

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: Theme.sp3

                            FieldGroup {
                                Layout.fillWidth: true
                                label: qsTr("Host")
                                description: qsTr("Hostname or IP address.")

                                PierTextField {
                                    id: hostField
                                    Layout.fillWidth: true
                                    placeholder: qsTr("example.com")
                                }
                            }

                            FieldGroup {
                                Layout.preferredWidth: 108
                                label: qsTr("Port")
                                description: qsTr("SSH")

                                PierTextField {
                                    id: portField
                                    Layout.fillWidth: true
                                    text: "22"
                                }
                            }
                        }

                        FieldGroup {
                            label: qsTr("Username")
                            description: qsTr("User account on the remote host.")

                            PierTextField {
                                id: userField
                                Layout.fillWidth: true
                                placeholder: qsTr("root")
                            }
                        }
                    }

                    Rectangle {
                        Layout.fillWidth: true
                        height: 1
                        color: Theme.borderSubtle
                    }

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: Theme.sp3

                        SectionLabel { text: qsTr("Authentication") }

                        Text {
                            Layout.fillWidth: true
                            text: qsTr("Choose how Pier-X should authenticate to this host.")
                            wrapMode: Text.WordWrap
                            font.family: Theme.fontUi
                            font.pixelSize: Theme.sizeSmall
                            color: Theme.textTertiary
                        }

                        SegmentedControl {
                            id: authMode
                            Layout.fillWidth: true
                            options: [qsTr("Password"), qsTr("Private key"), qsTr("SSH agent")]
                            currentIndex: 0
                        }

                        FieldGroup {
                            visible: root.passwordAuth
                            Layout.fillWidth: true
                            label: qsTr("Password")
                            description: qsTr("Stored securely in the system keychain.")

                            PierTextField {
                                id: passwordField
                                Layout.fillWidth: true
                                placeholder: qsTr("password")
                                password: true
                            }
                        }

                        ColumnLayout {
                            visible: root.keyAuth
                            Layout.fillWidth: true
                            spacing: Theme.sp3

                            FieldGroup {
                                Layout.fillWidth: true
                                label: qsTr("Private key file")
                                description: qsTr("Absolute path to an OpenSSH private key.")

                                RowLayout {
                                    Layout.fillWidth: true
                                    spacing: Theme.sp2

                                    PierTextField {
                                        id: keyPathField
                                        Layout.fillWidth: true
                                        placeholder: qsTr("~/.ssh/id_ed25519")
                                    }

                                    GhostButton {
                                        text: qsTr("Browse…")
                                        onClicked: keyFileDialog.open()
                                    }
                                }
                            }

                            FieldGroup {
                                Layout.fillWidth: true
                                label: qsTr("Passphrase")
                                description: qsTr("Optional. Leave empty for unencrypted keys.")

                                PierTextField {
                                    id: passphraseField
                                    Layout.fillWidth: true
                                    placeholder: qsTr("leave empty if unencrypted")
                                    password: true
                                }
                            }
                        }

                        Card {
                            Layout.fillWidth: true
                            visible: root.agentAuth
                            padding: Theme.sp3

                            ColumnLayout {
                                anchors.fill: parent
                                spacing: Theme.sp1

                                Text {
                                    Layout.fillWidth: true
                                    text: qsTr("SSH agent")
                                    font.family: Theme.fontUi
                                    font.pixelSize: Theme.sizeBody
                                    font.weight: Theme.weightMedium
                                    color: Theme.textPrimary
                                }

                                Text {
                                    Layout.fillWidth: true
                                    text: Qt.platform.os === "windows"
                                        ? qsTr("Uses Pageant or the Windows OpenSSH agent. Make sure your key is already loaded before connecting.")
                                        : qsTr("Uses the agent exposed at $SSH_AUTH_SOCK. Make sure your key is added first, for example with ssh-add.")
                                    wrapMode: Text.WordWrap
                                    font.family: Theme.fontUi
                                    font.pixelSize: Theme.sizeSmall
                                    color: Theme.textSecondary
                                }
                            }
                        }
                    }

                    Item { implicitHeight: Theme.sp1 }
                }
            }

            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: Theme.dialogFooterHeight
                color: Theme.bgChrome

                Rectangle {
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.top: parent.top
                    height: 1
                    color: Theme.borderSubtle
                }

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: Theme.sp5
                    anchors.rightMargin: Theme.sp5
                    spacing: Theme.sp2

                    Text {
                        Layout.fillWidth: true
                        text: qsTr("Host and username are required. Name is optional.")
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeSmall
                        color: Theme.textTertiary
                    }

                    GhostButton {
                        text: qsTr("Cancel")
                        onClicked: {
                            root.cancelled()
                            root.hide()
                        }
                    }

                    PrimaryButton {
                        text: qsTr("Connect")
                        enabled: hostField.text.trim().length > 0
                                 && userField.text.trim().length > 0
                                 && (
                                     (root.passwordAuth && passwordField.text.length > 0)
                                     || (root.keyAuth && keyPathField.text.trim().length > 0)
                                     || root.agentAuth
                                 )
                        onClicked: root.submit()
                    }
                }
            }
        }
    }

    FileDialog {
        id: keyFileDialog
        title: qsTr("Select SSH private key")
        fileMode: FileDialog.OpenFile
        onAccepted: {
            var url = selectedFile.toString()
            if (url.startsWith("file://"))
                url = url.substring(7)
            keyPathField.text = url
        }
    }

    component FieldGroup: ColumnLayout {
        property string label: ""
        property string description: ""
        default property alias controls: fieldSlot.data

        spacing: Theme.sp1_5

        Text {
            text: parent.label
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeCaption
            font.weight: Theme.weightMedium
            color: Theme.textPrimary
        }

        Text {
            visible: parent.description.length > 0
            text: parent.description
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeSmall
            color: Theme.textTertiary
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
        }

        ColumnLayout {
            id: fieldSlot
            Layout.fillWidth: true
            spacing: Theme.sp2
        }
    }
}
