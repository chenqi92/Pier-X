import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import Pier

// Modal dialog — collect SSH connection details and emit `saved` with the
// resulting object.
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
        open = true
        Qt.callLater(() => hostField.forceActiveFocus())
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

    ModalDialogShell {
        open: root.open
        dialogWidth: 560
        dialogHeight: 620
        title: qsTr("New SSH connection")
        subtitle: qsTr("Create a reusable host profile for SSH, SFTP, and service panels.")
        bodyPadding: 0
        onRequestClose: {
            root.cancelled()
            root.hide()
        }

        body: PierScrollView {
            id: formScroll
            anchors.fill: parent
            clip: true
            contentWidth: width

            Item {
                width: formScroll.width
                implicitHeight: form.implicitHeight + Theme.sp5 * 2

                ColumnLayout {
                    id: form
                    width: parent.width - Theme.sp5 * 2
                    anchors.horizontalCenter: parent.horizontalCenter
                    anchors.top: parent.top
                    anchors.topMargin: Theme.sp5
                    spacing: Theme.sp5

                    // ── Connection fields ──
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: Theme.sp4

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
                                Layout.preferredWidth: 100
                                label: qsTr("Port")

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

                    // ── Divider ──
                    Rectangle {
                        Layout.fillWidth: true
                        height: 1
                        color: Theme.borderSubtle
                    }

                    // ── Authentication ──
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: Theme.sp4

                        FieldGroup {
                            label: qsTr("Authentication")
                            description: qsTr("Choose how Pier-X should authenticate to this host.")
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
                            spacing: Theme.sp4

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

                        Text {
                            Layout.fillWidth: true
                            visible: root.agentAuth
                            text: Qt.platform.os === "windows"
                                ? qsTr("Uses Pageant or the Windows OpenSSH agent. Make sure your key is already loaded before connecting.")
                                : qsTr("Uses the agent exposed at $SSH_AUTH_SOCK. Make sure your key is added first, for example with ssh-add.")
                            wrapMode: Text.WordWrap
                            font.family: Theme.fontUi
                            font.pixelSize: Theme.sizeSmall
                            color: Theme.textSecondary
                        }
                    }

                    Item { implicitHeight: Theme.sp1 }
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
                    text: qsTr("Host and username are required. Name is optional.")
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeSmall
                    color: Theme.textTertiary
                    wrapMode: Text.WordWrap
                    Layout.alignment: Qt.AlignVCenter
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
        Layout.fillWidth: true
        Layout.minimumWidth: 0
        property string label: ""
        property string description: ""
        default property alias controls: fieldSlot.data

        spacing: Theme.sp1_5

        Text {
            text: parent.label
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeBody
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
