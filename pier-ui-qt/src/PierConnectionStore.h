// ─────────────────────────────────────────────────────────
// PierConnectionStore — QObject wrapper around the persisted
// connections JSON store
// ─────────────────────────────────────────────────────────
//
// Loads `connections.json` on construction (or first use) and
// exposes the entries as a QML-friendly list. Adding a
// connection appends to the in-memory list and writes the file
// atomically through the Rust C ABI.
//
// Schema mirrors pier_core::connections::ConnectionStore.
// Each connection has:
//
//   * name           QString — display label
//   * host           QString
//   * port           int
//   * username       QString
//   * credentialId   QString  (opaque keychain key)
//   * tags           QStringList
//
// The credentialId is the only field referencing a secret;
// the actual password lives in the OS keychain under that id
// and is looked up by the SSH session layer at connect time.
//
// Threading: every method on this class runs on the Qt main
// thread. The Rust load/save calls block briefly on file I/O —
// typical sub-millisecond on local disk. If profiling shows
// startup load latency in the future we can move that to a
// worker thread without changing the QML-facing surface.

#pragma once

#include <QAbstractListModel>
#include <QObject>
#include <QString>
#include <QStringList>
#include <QVariantMap>
#include <qqml.h>

#include <vector>

class PierConnectionStore : public QAbstractListModel
{
    Q_OBJECT
    QML_NAMED_ELEMENT(PierConnectionStore)

    // QAbstractListModel exposes rowCount(parent) but QML
    // bindings normally read `.count` as a property — expose it
    // explicitly so the Sidebar's empty-state binding and any
    // future "Connections (3)" header bindings work without
    // wrapper code on the QML side.
    Q_PROPERTY(int count READ count NOTIFY countChanged FINAL)

public:
    // QML-facing roles. Each maps to a column on the JSON shape.
    enum Roles {
        NameRole = Qt::UserRole + 1,
        HostRole,
        PortRole,
        UsernameRole,
        CredentialIdRole,
        KeyPathRole,
        PassphraseCredentialIdRole,
        UsesAgentRole,
        TagsRole
    };

    explicit PierConnectionStore(QObject *parent = nullptr);

    // QAbstractListModel interface.
    int rowCount(const QModelIndex &parent = QModelIndex()) const override;
    QVariant data(const QModelIndex &index, int role = Qt::DisplayRole) const override;
    QHash<int, QByteArray> roleNames() const override;

public slots:
    // Reload from disk. Called once at construction; QML can
    // call again after an external write (rare).
    void reload();

    // Append a new password-auth connection and persist
    // atomically. Returns true on success. The caller is
    // responsible for storing the matching keychain entry
    // first via PierCredentials.
    bool add(const QString &name, const QString &host, int port,
             const QString &username, const QString &credentialId);

    // Append a new key-auth connection and persist atomically.
    // `privateKeyPath` is an absolute on-disk path to the
    // OpenSSH-format private key file (NOT a secret — paths
    // can live in plaintext on disk). `passphraseCredentialId`
    // is empty for an unencrypted key, or a previously stored
    // keychain id holding the passphrase.
    bool addKey(const QString &name, const QString &host, int port,
                const QString &username,
                const QString &privateKeyPath,
                const QString &passphraseCredentialId);

    // Append a new agent-auth connection and persist
    // atomically. No credential fields at all — the OS SSH
    // agent holds the keys.
    bool addAgent(const QString &name, const QString &host, int port,
                  const QString &username);

    // Remove the connection at `index` and persist. Does NOT
    // delete the keychain entry — the caller is responsible
    // for that via PierCredentials::deleteEntry.
    bool removeAt(int index);

    // Returns the connection at `index` as a QVariantMap, or
    // an empty map if out of range. QML uses this to look up
    // the credential id and SSH params for a sidebar entry.
    QVariantMap get(int index) const;

    int count() const { return static_cast<int>(m_entries.size()); }

signals:
    void countChanged();

private:
    // POD struct mirroring one row of the JSON shape. Exactly
    // one of (credentialId, keyPath, usesAgent) is set per
    // row; the other two are empty / false. `passphraseCredentialId`
    // is only meaningful with `keyPath` set.
    struct Entry {
        QString name;
        QString host;
        int port = 22;
        QString username;
        QString credentialId;          // password-auth: keychain id of password
        QString keyPath;               // key-auth: absolute path to private key
        QString passphraseCredentialId; // key-auth: keychain id of passphrase (or empty)
        bool    usesAgent = false;     // agent-auth: system SSH agent
        QStringList tags;
    };

    // Insert + persist + roll-back-on-failure helper used by
    // both add() and addKey().
    bool appendEntry(Entry e);

    // Serialize the in-memory entries to JSON in the exact
    // shape expected by pier_core::connections::ConnectionStore,
    // then call pier_connections_save_json. Returns true on
    // success.
    bool persist();

    // Parse a JSON document loaded from pier_connections_load_json
    // and replace m_entries with its contents. Returns true on
    // success; on parse failure leaves m_entries untouched and
    // logs the error.
    bool ingestJson(const QByteArray &json);

    std::vector<Entry> m_entries;
};
