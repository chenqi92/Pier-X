#include "PierSqliteClient.h"
#include "pier_sqlite.h"

#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QMetaObject>

PierSqliteClient::PierSqliteClient(QObject *parent) : QObject(parent) {}

PierSqliteClient::~PierSqliteClient()
{
    close();
    for (auto &t : m_workers) if (t && t->joinable()) t->detach();
}

void PierSqliteClient::setStatus(Status s) { if (m_status == s) return; m_status = s; emit statusChanged(); }
void PierSqliteClient::setBusy(bool b) { if (m_busy == b) return; m_busy = b; emit busyChanged(); }

void PierSqliteClient::open(const QString &path)
{
    if (m_handle) close();
    if (path.isEmpty()) return;

    const quint64 id = ++m_nextRequestId;
    m_cancelFlag = std::make_shared<std::atomic<bool>>(false);
    setStatus(Loading); setBusy(true);

    std::string p = path.toStdString();
    QPointer<PierSqliteClient> self(this);
    auto cancel = m_cancelFlag;

    auto w = std::make_unique<std::thread>([self, cancel, id, p = std::move(p)]() {
        auto *h = pier_sqlite_open(p.c_str());
        QString err;
        if (!h) err = QStringLiteral("Failed to open database");
        if (!self || (cancel && cancel->load())) { if (h) pier_sqlite_free(h); return; }
        QMetaObject::invokeMethod(self.data(), "onOpenResult", Qt::QueuedConnection,
            Q_ARG(quint64, id), Q_ARG(void*, static_cast<void*>(h)), Q_ARG(QString, err));
    });
    m_workers.push_back(std::move(w));
}

void PierSqliteClient::onOpenResult(quint64 id, void *handle, const QString &error)
{
    if (id != m_nextRequestId) { if (handle) pier_sqlite_free(static_cast<::PierSqlite*>(handle)); return; }
    if (!handle) { m_errorMessage = error; setStatus(Failed); setBusy(false); return; }
    m_handle = static_cast<::PierSqlite*>(handle);
    m_dbPath = QString(); // set from open arg
    emit dbChanged();
    setStatus(Ready); setBusy(false);
    refreshTables();
}

void PierSqliteClient::refreshTables()
{
    if (!m_handle) return;
    const quint64 id = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierSqliteClient> self(this);
    auto cancel = m_cancelFlag;
    auto *h = m_handle;

    auto w = std::make_unique<std::thread>([self, cancel, id, h]() {
        char *json = pier_sqlite_list_tables(h);
        QString result = json ? QString::fromUtf8(json) : QStringLiteral("[]");
        if (json) pier_sqlite_free_string(json);
        if (!self || (cancel && cancel->load())) return;
        QMetaObject::invokeMethod(self.data(), "onTablesResult", Qt::QueuedConnection,
            Q_ARG(quint64, id), Q_ARG(QString, result));
    });
    m_workers.push_back(std::move(w));
}

void PierSqliteClient::onTablesResult(quint64 id, const QString &json)
{
    if (id != m_nextRequestId) return;
    QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8());
    QStringList list;
    if (doc.isArray()) for (const auto &v : doc.array()) if (v.isString()) list.append(v.toString());
    m_tables = list;
    emit tablesChanged();
    setBusy(false);
}

void PierSqliteClient::loadColumns(const QString &table)
{
    if (!m_handle || table.isEmpty()) return;
    const quint64 id = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierSqliteClient> self(this);
    auto cancel = m_cancelFlag;
    auto *h = m_handle;
    std::string t = table.toStdString();

    auto w = std::make_unique<std::thread>([self, cancel, id, h, t = std::move(t)]() {
        char *json = pier_sqlite_table_columns(h, t.c_str());
        QString result = json ? QString::fromUtf8(json) : QStringLiteral("[]");
        if (json) pier_sqlite_free_string(json);
        if (!self || (cancel && cancel->load())) return;
        QMetaObject::invokeMethod(self.data(), "onColumnsResult", Qt::QueuedConnection,
            Q_ARG(quint64, id), Q_ARG(QString, result));
    });
    m_workers.push_back(std::move(w));
}

void PierSqliteClient::onColumnsResult(quint64 id, const QString &json)
{
    if (id != m_nextRequestId) return;
    QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8());
    QVariantList list;
    if (doc.isArray()) for (const auto &v : doc.array()) list.append(v.toObject().toVariantMap());
    m_columns = list;
    emit columnsChanged();
    setBusy(false);
}

void PierSqliteClient::execute(const QString &sql)
{
    if (!m_handle || sql.isEmpty()) return;
    const quint64 id = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierSqliteClient> self(this);
    auto cancel = m_cancelFlag;
    auto *h = m_handle;
    std::string s = sql.toStdString();

    auto w = std::make_unique<std::thread>([self, cancel, id, h, s = std::move(s)]() {
        char *json = pier_sqlite_execute(h, s.c_str());
        QString result = json ? QString::fromUtf8(json) : QStringLiteral("{}");
        if (json) pier_sqlite_free_string(json);
        if (!self || (cancel && cancel->load())) return;
        QMetaObject::invokeMethod(self.data(), "onExecuteResult", Qt::QueuedConnection,
            Q_ARG(quint64, id), Q_ARG(QString, result));
    });
    m_workers.push_back(std::move(w));
}

void PierSqliteClient::onExecuteResult(quint64 id, const QString &json)
{
    if (id != m_nextRequestId) return;
    QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8());
    QJsonObject obj = doc.object();

    m_lastError = obj.value(QStringLiteral("error")).toString();
    m_lastElapsedMs = static_cast<qint64>(obj.value(QStringLiteral("elapsed_ms")).toDouble());

    QVariantList cols;
    for (const auto &v : obj.value(QStringLiteral("columns")).toArray())
        cols.append(v.toString());
    m_resultColumns = cols;

    QVariantList rows;
    for (const auto &r : obj.value(QStringLiteral("rows")).toArray()) {
        QVariantList row;
        for (const auto &c : r.toArray()) row.append(c.toString());
        rows.append(QVariant::fromValue(row));
    }
    m_resultRows = rows;

    emit resultChanged();
    setBusy(false);
}

void PierSqliteClient::close()
{
    if (m_cancelFlag) m_cancelFlag->store(true);
    if (m_handle) { pier_sqlite_free(m_handle); m_handle = nullptr; }
    m_tables.clear(); m_columns.clear();
    m_resultColumns.clear(); m_resultRows.clear();
    m_dbPath.clear();
    setStatus(Idle);
    emit dbChanged(); emit tablesChanged();
}
