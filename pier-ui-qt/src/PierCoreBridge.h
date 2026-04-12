// ─────────────────────────────────────────────────────────
// PierCoreBridge — thin C++ wrapper around pier-core's C ABI
// ─────────────────────────────────────────────────────────
//
// This is the first Rust ↔ Qt integration point in Pier-X. It exposes
// the handful of pier-core C functions as a read-only QML singleton
// named `PierCore`, so QML can write things like:
//
//     text: "pier-core " + PierCore.version + " (" + PierCore.buildInfo + ")"
//
// Only read-only properties live here for now. Once M2 needs signals,
// slots, async methods, or QObjects with state, this file is the
// obvious place to grow a cxx-qt-generated layer alongside it.

#pragma once

#include <QObject>
#include <QString>
#include <qqml.h>

class PierCoreBridge : public QObject
{
    Q_OBJECT
    QML_NAMED_ELEMENT(PierCore)
    QML_SINGLETON

    // pier-core crate version, e.g. "0.1.0"
    Q_PROPERTY(QString version READ version CONSTANT FINAL)

    // "<version> (release|debug)"
    Q_PROPERTY(QString buildInfo READ buildInfo CONSTANT FINAL)

    // Qt runtime version string from qVersion(), e.g. "6.11.0"
    Q_PROPERTY(QString qtVersion READ qtVersion CONSTANT FINAL)

    // Process working directory at startup (for Git panel).
    Q_PROPERTY(QString workingDirectory READ workingDirectory CONSTANT FINAL)

public:
    explicit PierCoreBridge(QObject *parent = nullptr);

    QString version() const;
    QString buildInfo() const;
    QString qtVersion() const;
    QString workingDirectory() const;

    // Returns true if pier-core was built with the named feature.
    Q_INVOKABLE bool hasFeature(const QString &name) const;

    // Returns combined local shell history (~/.zsh_history, ~/.bash_history).
    Q_INVOKABLE QStringList localHistory() const;
};
