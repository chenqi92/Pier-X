// ─────────────────────────────────────────────────────────
// PierUpdaterBridge — QML singleton for auto-update control
// ─────────────────────────────────────────────────────────
//
// Wraps the PierUpdater namespace functions into a QML-accessible
// singleton named `PierUpdate`.  The Settings dialog binds to
// `available`, `autoCheck`, and `checkForUpdates()`.
//
// On platforms without an update framework (Linux), `available`
// returns false and the UI hides the entire Updates section.

#pragma once

#include <QObject>
#include <qqml.h>

class PierUpdaterBridge : public QObject
{
    Q_OBJECT
    QML_NAMED_ELEMENT(PierUpdate)
    QML_SINGLETON

    /// True on macOS (Sparkle) and Windows (WinSparkle).
    Q_PROPERTY(bool available READ available CONSTANT FINAL)

    /// Whether background update checks are enabled.
    Q_PROPERTY(bool autoCheck READ autoCheck WRITE setAutoCheck NOTIFY autoCheckChanged FINAL)

public:
    explicit PierUpdaterBridge(QObject *parent = nullptr);

    bool available() const;
    bool autoCheck() const;
    void setAutoCheck(bool enabled);

    /// Open the native update-check dialog.
    Q_INVOKABLE void checkForUpdates();

signals:
    void autoCheckChanged();
};
