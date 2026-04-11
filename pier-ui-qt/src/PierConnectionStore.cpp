#include "PierConnectionStore.h"

#include "pier_connections.h"

#include <QByteArray>
#include <QDebug>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonValue>

PierConnectionStore::PierConnectionStore(QObject *parent)
    : QAbstractListModel(parent)
{
    reload();
}

int PierConnectionStore::rowCount(const QModelIndex &parent) const
{
    if (parent.isValid()) {
        return 0;
    }
    return static_cast<int>(m_entries.size());
}

QVariant PierConnectionStore::data(const QModelIndex &index, int role) const
{
    if (!index.isValid()) {
        return {};
    }
    const int row = index.row();
    if (row < 0 || row >= static_cast<int>(m_entries.size())) {
        return {};
    }
    const Entry &e = m_entries[static_cast<size_t>(row)];
    switch (role) {
    case NameRole:         return e.name;
    case HostRole:         return e.host;
    case PortRole:         return e.port;
    case UsernameRole:     return e.username;
    case CredentialIdRole: return e.credentialId;
    case TagsRole:         return e.tags;
    default:               return {};
    }
}

QHash<int, QByteArray> PierConnectionStore::roleNames() const
{
    return {
        { NameRole,         "name" },
        { HostRole,         "host" },
        { PortRole,         "port" },
        { UsernameRole,     "username" },
        { CredentialIdRole, "credentialId" },
        { TagsRole,         "tags" }
    };
}

void PierConnectionStore::reload()
{
    char *raw = pier_connections_load_json();
    if (!raw) {
        // No data dir or malformed file. Treat as empty store —
        // the next add() persists a fresh document on top.
        beginResetModel();
        m_entries.clear();
        endResetModel();
        emit countChanged();
        return;
    }
    // QByteArray ctor copies the C string up to the first NUL —
    // pier_connections_free_json then releases the Rust-side
    // buffer immediately, so the QByteArray below outlives it.
    const QByteArray json(raw);
    pier_connections_free_json(raw);

    beginResetModel();
    m_entries.clear();
    if (!ingestJson(json)) {
        qWarning() << "PierConnectionStore: failed to ingest connections JSON";
    }
    endResetModel();
    emit countChanged();
}

bool PierConnectionStore::add(const QString &name, const QString &host, int port,
                              const QString &username, const QString &credentialId)
{
    if (name.isEmpty() || host.isEmpty() || username.isEmpty() || credentialId.isEmpty()) {
        qWarning() << "PierConnectionStore::add rejected empty field";
        return false;
    }
    Entry e;
    e.name = name;
    e.host = host;
    e.port = port > 0 ? port : 22;
    e.username = username;
    e.credentialId = credentialId;

    const int row = static_cast<int>(m_entries.size());
    beginInsertRows(QModelIndex(), row, row);
    m_entries.push_back(std::move(e));
    endInsertRows();

    if (!persist()) {
        // Roll back the in-memory insert so model state matches
        // on-disk state. The QML side will not see the row.
        beginRemoveRows(QModelIndex(), row, row);
        m_entries.pop_back();
        endRemoveRows();
        return false;
    }
    emit countChanged();
    return true;
}

bool PierConnectionStore::removeAt(int index)
{
    if (index < 0 || index >= static_cast<int>(m_entries.size())) {
        return false;
    }
    beginRemoveRows(QModelIndex(), index, index);
    Entry removed = std::move(m_entries[static_cast<size_t>(index)]);
    m_entries.erase(m_entries.begin() + index);
    endRemoveRows();

    if (!persist()) {
        // Persist failed — restore the in-memory entry so
        // model and disk stay consistent.
        beginInsertRows(QModelIndex(), index, index);
        m_entries.insert(m_entries.begin() + index, std::move(removed));
        endInsertRows();
        return false;
    }
    emit countChanged();
    return true;
}

QVariantMap PierConnectionStore::get(int index) const
{
    if (index < 0 || index >= static_cast<int>(m_entries.size())) {
        return {};
    }
    const Entry &e = m_entries[static_cast<size_t>(index)];
    QVariantMap m;
    m["name"]         = e.name;
    m["host"]         = e.host;
    m["port"]         = e.port;
    m["username"]     = e.username;
    m["credentialId"] = e.credentialId;
    m["tags"]         = e.tags;
    return m;
}

bool PierConnectionStore::ingestJson(const QByteArray &json)
{
    QJsonParseError err {};
    const QJsonDocument doc = QJsonDocument::fromJson(json, &err);
    if (err.error != QJsonParseError::NoError) {
        qWarning() << "PierConnectionStore JSON parse error:" << err.errorString();
        return false;
    }
    if (!doc.isObject()) {
        qWarning() << "PierConnectionStore JSON root is not an object";
        return false;
    }
    const QJsonObject root = doc.object();
    const QJsonArray arr = root.value(QStringLiteral("connections")).toArray();
    for (const QJsonValue &v : arr) {
        if (!v.isObject()) continue;
        const QJsonObject obj = v.toObject();
        Entry e;
        e.name = obj.value(QStringLiteral("name")).toString();
        e.host = obj.value(QStringLiteral("host")).toString();
        e.port = obj.value(QStringLiteral("port")).toInt(22);
        e.username = obj.value(QStringLiteral("user")).toString();
        // The Rust SshConfig::AuthMethod is a tagged union. Only
        // the keychain_password variant carries a credential id;
        // any other variant on disk we treat as "no credential
        // wired", which means clicking the entry can't reconnect
        // until M3c3 lands the other auth methods.
        const QJsonObject auth = obj.value(QStringLiteral("auth")).toObject();
        const QString kind = auth.value(QStringLiteral("kind")).toString();
        if (kind == QStringLiteral("keychain_password")) {
            e.credentialId = auth.value(QStringLiteral("credential_id")).toString();
        }
        // Tags
        const QJsonArray tags = obj.value(QStringLiteral("tags")).toArray();
        for (const QJsonValue &t : tags) {
            if (t.isString()) {
                e.tags.append(t.toString());
            }
        }
        if (!e.name.isEmpty() && !e.host.isEmpty()) {
            m_entries.push_back(std::move(e));
        }
    }
    return true;
}

bool PierConnectionStore::persist()
{
    // Build the exact JSON shape pier_core::connections::ConnectionStore
    // expects: { version: 1, connections: [ { name, host, port, user,
    // auth: { kind, credential_id }, connect_timeout_secs, tags } ] }.
    QJsonArray arr;
    for (const Entry &e : m_entries) {
        QJsonObject auth;
        auth[QStringLiteral("kind")] = QStringLiteral("keychain_password");
        auth[QStringLiteral("credential_id")] = e.credentialId;

        QJsonArray tagArr;
        for (const QString &t : e.tags) {
            tagArr.append(t);
        }

        QJsonObject obj;
        obj[QStringLiteral("name")] = e.name;
        obj[QStringLiteral("host")] = e.host;
        obj[QStringLiteral("port")] = e.port;
        obj[QStringLiteral("user")] = e.username;
        obj[QStringLiteral("auth")] = auth;
        obj[QStringLiteral("connect_timeout_secs")] = 10;
        obj[QStringLiteral("tags")] = tagArr;
        arr.append(obj);
    }
    QJsonObject root;
    root[QStringLiteral("version")] = 1;
    root[QStringLiteral("connections")] = arr;

    const QByteArray json = QJsonDocument(root).toJson(QJsonDocument::Compact);
    const int32_t rc = pier_connections_save_json(json.constData());
    if (rc != 0) {
        qWarning() << "pier_connections_save_json failed rc=" << rc;
        return false;
    }
    return true;
}
