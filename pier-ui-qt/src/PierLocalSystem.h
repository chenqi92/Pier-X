// ─────────────────────────────────────────────────────────
// PierLocalSystem — tiny native helpers for local file UX
// ─────────────────────────────────────────────────────────
//
// Keeps local-file shell actions inside Qt/C++ instead of sprinkling
// platform-specific behavior through QML. The file pane uses this for
// clipboard access and for revealing paths in Finder / Explorer.

#pragma once

#include <QObject>
#include <QString>
#include <qqml.h>

class PierLocalSystem : public QObject
{
    Q_OBJECT
    QML_NAMED_ELEMENT(PierLocalSystem)
    QML_SINGLETON

public:
    explicit PierLocalSystem(QObject *parent = nullptr);

    Q_INVOKABLE bool copyText(const QString &text) const;
    Q_INVOKABLE QString readText() const;
    Q_INVOKABLE bool openPath(const QString &path) const;
    Q_INVOKABLE bool revealPath(const QString &path) const;
    Q_INVOKABLE bool initGitRepository(const QString &path) const;
};
