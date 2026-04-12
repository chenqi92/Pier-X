#include "PierPostgresClient.h"
#include "pier_postgres.h"

#include <QByteArray>
#include <QDebug>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QMetaObject>

// ── PierPgResultModel ────────────────────────────────────

PierPgResultModel::PierPgResultModel(QObject *parent)
    : QAbstractTableModel(parent) {}

int PierPgResultModel::rowCount(const QModelIndex &parent) const
{ if (parent.isValid()) return 0; return static_cast<int>(m_rows.size()); }

int PierPgResultModel::columnCount(const QModelIndex &parent) const
{ if (parent.isValid()) return 0; return m_columns.size(); }

QVariant PierPgResultModel::data(const QModelIndex &index, int role) const
{
    if (!index.isValid()) return {};
    const int r = index.row(), c = index.column();
    if (r < 0 || r >= static_cast<int>(m_rows.size())) return {};
    if (c < 0 || c >= static_cast<int>(m_rows[r].size())) return {};
    const Cell &cell = m_rows[static_cast<size_t>(r)][static_cast<size_t>(c)];
    switch (role) {
    case Qt::DisplayRole: case DisplayRole:
        return cell.isNull ? QVariant() : QVariant(cell.value);
    case IsNullRole: return cell.isNull;
    default: return {};
    }
}

QVariant PierPgResultModel::headerData(int section, Qt::Orientation o, int role) const
{
    if ((role != Qt::DisplayRole && role != DisplayRole)
        || o != Qt::Horizontal
        || section < 0 || section >= m_columns.size()) return {};
    return m_columns.at(section);
}

QHash<int, QByteArray> PierPgResultModel::roleNames() const
{ return {{ DisplayRole, "display" }, { IsNullRole, "isNull" }}; }

void PierPgResultModel::resetWith(const QStringList &columns,
                                   const std::vector<std::vector<QString>> &rows,
                                   const std::vector<std::vector<bool>> &nulls)
{
    beginResetModel();
    m_columns = columns;
    m_rows.clear();
    m_rows.reserve(rows.size());
    for (size_t r = 0; r < rows.size(); ++r) {
        const auto &rv = rows[r];
        const auto &rn = (r < nulls.size()) ? nulls[r] : std::vector<bool>{};
        std::vector<Cell> row;
        row.reserve(rv.size());
        for (size_t c = 0; c < rv.size(); ++c) {
            row.push_back({ rv[c], c < rn.size() && rn[c] });
        }
        m_rows.push_back(std::move(row));
    }
    endResetModel();
}

void PierPgResultModel::clearAll()
{ beginResetModel(); m_columns.clear(); m_rows.clear(); endResetModel(); }

// ── PierPostgresClient ───────────────────────────────────

PierPostgresClient::PierPostgresClient(QObject *parent)
    : QObject(parent), m_resultModel(new PierPgResultModel(this)) {}

PierPostgresClient::~PierPostgresClient()
{
    stop();
    for (auto &t : m_workers) { if (t && t->joinable()) t->detach(); }
}

void PierPostgresClient::setStatus(Status s)
{ if (m_status != s) { m_status = s; emit statusChanged(); } }

void PierPostgresClient::setBusy(bool b)
{ if (m_busy != b) { m_busy = b; emit busyChanged(); } }

bool PierPostgresClient::connectTo(const QString &host, int port,
                                    const QString &user, const QString &password,
                                    const QString &database)
{
    if (m_handle || m_status == Connecting) return false;
    if (host.isEmpty() || user.isEmpty() || port <= 0 || port > 65535) return false;

    const quint64 requestId = ++m_nextRequestId;
    m_cancelFlag = std::make_shared<std::atomic<bool>>(false);
    m_errorMessage.clear();
    m_target = database.isEmpty()
        ? QStringLiteral("%1@%2:%3").arg(user, host).arg(port)
        : QStringLiteral("%1@%2:%3/%4").arg(user, host).arg(port).arg(database);
    setStatus(Connecting);
    setBusy(true);

    std::string h = host.toStdString(), u = user.toStdString(),
                p = password.toStdString(), d = database.toStdString();
    const uint16_t pt = static_cast<uint16_t>(port);
    QPointer<PierPostgresClient> self(this);
    auto cf = m_cancelFlag;

    auto worker = std::make_unique<std::thread>([self, cf, requestId,
        h=std::move(h), u=std::move(u), p=std::move(p), d=std::move(d), pt
    ]() mutable {
        const char *pp = p.empty() ? nullptr : p.c_str();
        const char *dp = d.empty() ? nullptr : d.c_str();
        ::PierPostgres *handle = pier_postgres_open(h.c_str(), pt, u.c_str(), pp, dp);
        QString err;
        if (!handle) err = QStringLiteral("PostgreSQL connect failed (see log)");
        if (!self || (cf && cf->load())) { if (handle) pier_postgres_free(handle); return; }
        QMetaObject::invokeMethod(self.data(), "onConnectResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(void*, static_cast<void*>(handle)), Q_ARG(QString, err));
    });
    m_workers.push_back(std::move(worker));
    return true;
}

void PierPostgresClient::onConnectResult(quint64 rid, void *handle, const QString &error)
{
    if (rid != m_nextRequestId) { if (handle) pier_postgres_free(static_cast<::PierPostgres*>(handle)); return; }
    setBusy(false);
    if (!handle) { m_errorMessage = error.isEmpty() ? QStringLiteral("PostgreSQL connect failed") : error; setStatus(Failed); return; }
    m_handle = static_cast<::PierPostgres*>(handle);
    setStatus(Connected);
    refreshDatabases();
}

void PierPostgresClient::execute(const QString &sql)
{
    if (!m_handle || sql.isEmpty()) return;
    const quint64 rid = ++m_nextRequestId;
    setBusy(true);
    std::string s = sql.toStdString();
    ::PierPostgres *h = m_handle;
    QPointer<PierPostgresClient> self(this);
    auto cf = m_cancelFlag;
    auto worker = std::make_unique<std::thread>([self, cf, rid, h, s=std::move(s)]() mutable {
        char *json = pier_postgres_execute(h, s.c_str());
        QString js;
        if (json) { js = QString::fromUtf8(json); pier_postgres_free_string(json); }
        if (!self || (cf && cf->load())) return;
        QMetaObject::invokeMethod(self.data(), "onExecuteResult", Qt::QueuedConnection,
            Q_ARG(quint64, rid), Q_ARG(QString, js));
    });
    m_workers.push_back(std::move(worker));
}

void PierPostgresClient::onExecuteResult(quint64, const QString &json)
{
    setBusy(false);
    if (json.isEmpty()) { m_lastError = QStringLiteral("execute failed"); m_resultModel->clearAll(); emit resultChanged(); return; }
    ingestExecuteJson(json);
}

void PierPostgresClient::ingestExecuteJson(const QString &json)
{
    QJsonParseError pe{};
    const QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8(), &pe);
    if (pe.error != QJsonParseError::NoError || !doc.isObject()) return;
    const QJsonObject obj = doc.object();
    m_lastError = obj.value(QStringLiteral("error")).toString();
    m_lastAffectedRows = static_cast<qint64>(obj.value(QStringLiteral("affected_rows")).toDouble());
    m_lastElapsedMs = static_cast<qint64>(obj.value(QStringLiteral("elapsed_ms")).toDouble());
    m_lastTruncated = obj.value(QStringLiteral("truncated")).toBool();
    QStringList cols;
    for (const auto &v : obj.value(QStringLiteral("columns")).toArray()) cols.append(v.toString());
    std::vector<std::vector<QString>> rows;
    std::vector<std::vector<bool>> nulls;
    for (const auto &rv : obj.value(QStringLiteral("rows")).toArray()) {
        if (!rv.isArray()) continue;
        std::vector<QString> rc; std::vector<bool> rn;
        for (const auto &cv : rv.toArray()) {
            if (cv.isNull()) { rc.emplace_back(); rn.push_back(true); }
            else { rc.push_back(cv.toString()); rn.push_back(false); }
        }
        rows.push_back(std::move(rc)); nulls.push_back(std::move(rn));
    }
    m_resultModel->resetWith(cols, rows, nulls);
    emit resultChanged();
}

void PierPostgresClient::refreshDatabases()
{
    if (!m_handle) return;
    const quint64 rid = ++m_nextRequestId;
    setBusy(true);
    ::PierPostgres *h = m_handle;
    QPointer<PierPostgresClient> self(this);
    auto cf = m_cancelFlag;
    auto worker = std::make_unique<std::thread>([self, cf, rid, h]() {
        char *json = pier_postgres_list_databases(h);
        QString js;
        if (json) { js = QString::fromUtf8(json); pier_postgres_free_string(json); }
        if (!self || (cf && cf->load())) return;
        QMetaObject::invokeMethod(self.data(), "onDatabasesResult", Qt::QueuedConnection,
            Q_ARG(quint64, rid), Q_ARG(QString, js));
    });
    m_workers.push_back(std::move(worker));
}

void PierPostgresClient::onDatabasesResult(quint64, const QString &json)
{
    setBusy(false);
    if (json.isEmpty()) return;
    ingestDatabasesJson(json);
}

void PierPostgresClient::ingestDatabasesJson(const QString &json)
{
    QJsonParseError pe{};
    const QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8(), &pe);
    if (pe.error != QJsonParseError::NoError || !doc.isArray()) return;
    QStringList dbs;
    for (const auto &v : doc.array()) dbs.append(v.toString());
    m_databases = std::move(dbs);
    emit databasesChanged();
}

void PierPostgresClient::refreshTables(const QString &schema)
{
    if (!m_handle) return;
    const quint64 rid = ++m_nextRequestId;
    setBusy(true);
    std::string s = schema.toStdString();
    ::PierPostgres *h = m_handle;
    QPointer<PierPostgresClient> self(this);
    auto cf = m_cancelFlag;
    auto worker = std::make_unique<std::thread>([self, cf, rid, h, s=std::move(s)]() mutable {
        const char *sp = s.empty() ? nullptr : s.c_str();
        char *json = pier_postgres_list_tables(h, sp);
        QString js;
        if (json) { js = QString::fromUtf8(json); pier_postgres_free_string(json); }
        if (!self || (cf && cf->load())) return;
        QMetaObject::invokeMethod(self.data(), "onTablesResult", Qt::QueuedConnection,
            Q_ARG(quint64, rid), Q_ARG(QString, js));
    });
    m_workers.push_back(std::move(worker));
}

void PierPostgresClient::onTablesResult(quint64, const QString &json)
{
    setBusy(false);
    if (json.isEmpty()) return;
    ingestTablesJson(json);
}

void PierPostgresClient::ingestTablesJson(const QString &json)
{
    QJsonParseError pe{};
    const QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8(), &pe);
    if (pe.error != QJsonParseError::NoError || !doc.isArray()) return;
    QStringList tables;
    for (const auto &v : doc.array()) tables.append(v.toString());
    m_tables = std::move(tables);
    emit tablesChanged();
}

void PierPostgresClient::refreshColumns(const QString &schema, const QString &table)
{
    if (!m_handle || table.isEmpty()) return;
    const quint64 rid = ++m_nextRequestId;
    setBusy(true);
    std::string s = schema.toStdString(), t = table.toStdString();
    ::PierPostgres *h = m_handle;
    QPointer<PierPostgresClient> self(this);
    auto cf = m_cancelFlag;
    auto worker = std::make_unique<std::thread>([self, cf, rid, h, s=std::move(s), t=std::move(t)]() mutable {
        const char *sp = s.empty() ? nullptr : s.c_str();
        char *json = pier_postgres_list_columns(h, sp, t.c_str());
        QString js;
        if (json) { js = QString::fromUtf8(json); pier_postgres_free_string(json); }
        if (!self || (cf && cf->load())) return;
        QMetaObject::invokeMethod(self.data(), "onColumnsResult", Qt::QueuedConnection,
            Q_ARG(quint64, rid), Q_ARG(QString, js));
    });
    m_workers.push_back(std::move(worker));
}

void PierPostgresClient::onColumnsResult(quint64, const QString &json)
{
    setBusy(false);
    if (json.isEmpty()) return;
    ingestColumnsJson(json);
}

void PierPostgresClient::ingestColumnsJson(const QString &json)
{
    QJsonParseError pe{};
    const QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8(), &pe);
    if (pe.error != QJsonParseError::NoError || !doc.isArray()) return;
    QVariantList cols;
    for (const auto &v : doc.array()) {
        if (!v.isObject()) continue;
        const QJsonObject o = v.toObject();
        QVariantMap m;
        m.insert(QStringLiteral("name"), o.value(QStringLiteral("name")).toString());
        m.insert(QStringLiteral("type"), o.value(QStringLiteral("column_type")).toString());
        m.insert(QStringLiteral("nullable"), o.value(QStringLiteral("nullable")).toBool());
        m.insert(QStringLiteral("key"), o.value(QStringLiteral("key")).toString());
        m.insert(QStringLiteral("defaultValue"), o.value(QStringLiteral("default_value")).toVariant());
        m.insert(QStringLiteral("extra"), o.value(QStringLiteral("extra")).toString());
        cols.push_back(m);
    }
    m_columns = std::move(cols);
    emit columnsChanged();
}

void PierPostgresClient::stop()
{
    if (m_cancelFlag) m_cancelFlag->store(true);
    ++m_nextRequestId;
    if (m_handle) { pier_postgres_free(m_handle); m_handle = nullptr; }
    m_resultModel->clearAll();
    if (!m_databases.isEmpty()) { m_databases.clear(); emit databasesChanged(); }
    if (!m_tables.isEmpty()) { m_tables.clear(); emit tablesChanged(); }
    if (!m_columns.isEmpty()) { m_columns.clear(); emit columnsChanged(); }
    if (m_status != Idle) { m_errorMessage.clear(); m_target.clear(); setStatus(Idle); }
    setBusy(false);
}
