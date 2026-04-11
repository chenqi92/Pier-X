// ─────────────────────────────────────────────────────────
// PierCredentials — write-only QML wrapper for the OS keyring
// ─────────────────────────────────────────────────────────
//
// Exposes the credentials half of the M3c2 FFI as a QML
// singleton. The dialog calls `set(id, value)` once at
// connect-and-save time; the sidebar reconnect path doesn't
// touch this class at all (the password lives entirely in
// keychain after that point and is read by the Rust SSH
// session layer, never by C++).
//
// `deleteEntry` is exposed so a future "Delete saved
// connection" UI can wipe the keychain entry alongside the
// PierConnectionStore::removeAt call.
//
// Read is deliberately NOT exposed. C++ never sees a stored
// password back. See pier_credentials.h for the rationale.

#pragma once

#include <QObject>
#include <QString>
#include <qqml.h>

class PierCredentials : public QObject
{
    Q_OBJECT
    QML_NAMED_ELEMENT(PierCredentials)
    QML_SINGLETON

public:
    explicit PierCredentials(QObject *parent = nullptr);

    // QML-callable. Returns true on success.
    Q_INVOKABLE bool setEntry(const QString &id, const QString &value);

    // QML-callable. Returns true on success (or if the entry
    // didn't exist — that's defined as success by the Rust
    // layer so the QML side doesn't have to special-case it).
    Q_INVOKABLE bool deleteEntry(const QString &id);

    // Generate a fresh, unguessable credential id of the form
    // "pier-x.<uuid>". The QML dialog calls this once per new
    // connection so the same plaintext password under two
    // different connections gets two different keychain
    // entries (and one delete cleanly removes one entry).
    Q_INVOKABLE QString freshId() const;
};
