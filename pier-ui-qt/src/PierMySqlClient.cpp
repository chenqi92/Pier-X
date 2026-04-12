#include "PierMySqlClient.h"

#include "pier_mysql.h"

#include <QByteArray>
#include <QDebug>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonValue>
#include <QMetaObject>

// ─── PierMySqlResultModel ────────────────────────────────

PierMySqlResultModel::PierMySqlResultModel(QObject *parent)
    : QAbstractTableModel(parent)
{
}

int PierMySqlResultModel::rowCount(const QModelIndex &parent) const
{
    if (parent.isValid()) return 0;
    return static_cast<int>(m_rows.size());
}

int PierMySqlResultModel::columnCount(const QModelIndex &parent) const
{
    if (parent.isValid()) return 0;
    return m_columns.size();
}

QVariant PierMySqlResultModel::data(const QModelIndex &index, int role) const
{
    if (!index.isValid()) return {};
    const int r = index.row();
    const int c = index.column();
    if (r < 0 || r >= static_cast<int>(m_rows.size())) return {};
    if (c < 0 || c >= static_cast<int>(m_rows[r].size())) return {};
    const Cell &cell = m_rows[static_cast<size_t>(r)][static_cast<size_t>(c)];
    switch (role) {
    case Qt::DisplayRole:
    case DisplayRole:
        return cell.isNull ? QVariant() : QVariant(cell.value);
    case IsNullRole:
        return cell.isNull;
    default:
        return {};
    }
}

QVariant PierMySqlResultModel::headerData(int section, Qt::Orientation orientation, int role) const
{
    if (role != Qt::DisplayRole && role != DisplayRole) return {};
    if (orientation == Qt::Horizontal
        && section >= 0 && section < m_columns.size()) {
        return m_columns.at(section);
    }
    return {};
}

QHash<int, QByteArray> PierMySqlResultModel::roleNames() const
{
    return {
        { DisplayRole, "display" },
        { IsNullRole,  "isNull" }
    };
}

void PierMySqlResultModel::resetWith(const QStringList &columns,
                                      const std::vector<std::vector<QString>> &rows,
                                      const std::vector<std::vector<bool>> &nulls)
{
    beginResetModel();
    m_columns = columns;
    m_rows.clear();
    m_rows.reserve(rows.size());
    for (size_t r = 0; r < rows.size(); ++r) {
        const auto &rowVals = rows[r];
        const auto &rowNulls = (r < nulls.size()) ? nulls[r] : std::vector<bool>{};
        std::vector<Cell> row;
        row.reserve(rowVals.size());
        for (size_t c = 0; c < rowVals.size(); ++c) {
            Cell cell;
            cell.value  = rowVals[c];
            cell.isNull = (c < rowNulls.size()) ? rowNulls[c] : false;
            row.push_back(std::move(cell));
        }
        m_rows.push_back(std::move(row));
    }
    endResetModel();
}

void PierMySqlResultModel::clearAll()
{
    beginResetModel();
    m_columns.clear();
    m_rows.clear();
    endResetModel();
}


// ─── PierMySqlClient ─────────────────────────────────────

PierMySqlClient::PierMySqlClient(QObject *parent)
    : QObject(parent)
    , m_resultModel(new PierMySqlResultModel(this))
{
}

PierMySqlClient::~PierMySqlClient()
{
    stop();
    for (auto &t : m_workers) {
        if (t && t->joinable()) {
            t->detach();
        }
    }
}

void PierMySqlClient::setStatus(Status s)
{
    if (m_status == s) return;
    m_status = s;
    emit statusChanged();
}

void PierMySqlClient::setBusy(bool b)
{
    if (m_busy == b) return;
    m_busy = b;
    emit busyChanged();
}

bool PierMySqlClient::connectTo(const QString &host, int port,
                                 const QString &user, const QString &password,
                                 const QString &database)
{
    return connectInternal(host, port, user, password, database, false);
}

bool PierMySqlClient::connectToWithCredential(
    const QString &host,
    int port,
    const QString &user,
    const QString &credentialId,
    const QString &database)
{
    return connectInternal(host, port, user, credentialId, database, true);
}

bool PierMySqlClient::connectInternal(
    const QString &host,
    int port,
    const QString &user,
    const QString &secret,
    const QString &database,
    bool useCredential)
{
    if (m_handle || m_status == Connecting) {
        qWarning() << "PierMySqlClient::connectTo called on already-connected session";
        return false;
    }
    if (host.isEmpty() || user.isEmpty() || port <= 0 || port > 65535
        || (useCredential && secret.isEmpty())) {
        return false;
    }

    const quint64 requestId = ++m_nextRequestId;
    m_cancelFlag = std::make_shared<std::atomic<bool>>(false);
    m_errorMessage.clear();
    m_target = database.isEmpty()
                ? QStringLiteral("%1@%2:%3").arg(user, host).arg(port)
                : QStringLiteral("%1@%2:%3/%4").arg(user, host).arg(port).arg(database);
    setStatus(Connecting);
    setBusy(true);

    std::string hostStd = host.toStdString();
    std::string userStd = user.toStdString();
    std::string secretStd = secret.toStdString();
    std::string dbStd = database.toStdString();
    const uint16_t portU16 = static_cast<uint16_t>(port);
    const bool credentialMode = useCredential;

    QPointer<PierMySqlClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;

    auto worker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId,
        hostStd = std::move(hostStd),
        userStd = std::move(userStd),
        secretStd = std::move(secretStd),
        dbStd = std::move(dbStd),
        portU16,
        credentialMode
    ]() mutable {
        const char *secretPtr = secretStd.empty() ? nullptr : secretStd.c_str();
        const char *dbPtr   = dbStd.empty()   ? nullptr : dbStd.c_str();

        ::PierMysql *h = credentialMode
            ? pier_mysql_open_with_credential(
                hostStd.c_str(),
                portU16,
                userStd.c_str(),
                secretPtr,
                dbPtr)
            : pier_mysql_open(
                hostStd.c_str(),
                portU16,
                userStd.c_str(),
                secretPtr,
                dbPtr);

        QString err;
        if (!h) err = QStringLiteral("MySQL connect failed (see log)");

        const bool cancelled = cancelFlag && cancelFlag->load();
        if (!selfWeak || cancelled) {
            if (h) pier_mysql_free(h);
            return;
        }
        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onConnectResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(void *, static_cast<void *>(h)),
            Q_ARG(QString, err));
    });
    m_workers.push_back(std::move(worker));
    return true;
}

void PierMySqlClient::onConnectResult(quint64 requestId, void *handle, const QString &error)
{
    if (requestId != m_nextRequestId) {
        if (handle) pier_mysql_free(static_cast<::PierMysql *>(handle));
        return;
    }
    if (!handle) {
        m_errorMessage = error.isEmpty() ? QStringLiteral("MySQL connect failed") : error;
        setStatus(Failed);
        setBusy(false);
        return;
    }
    m_handle = static_cast<::PierMysql *>(handle);
    setStatus(Connected);
    setBusy(false);
    // Auto-refresh the database list so the UI has a schema picker.
    refreshDatabases();
}

void PierMySqlClient::execute(const QString &sql)
{
    if (!m_handle || sql.isEmpty()) return;
    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);

    std::string sqlStd = sql.toStdString();
    ::PierMysql *h = m_handle;
    QPointer<PierMySqlClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;

    auto worker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId, h,
        sqlStd = std::move(sqlStd)
    ]() mutable {
        char *json = pier_mysql_execute(h, sqlStd.c_str());
        QString jsonStr;
        if (json) {
            jsonStr = QString::fromUtf8(json);
            pier_mysql_free_string(json);
        }
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onExecuteResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(QString, jsonStr));
    });
    m_workers.push_back(std::move(worker));
}

void PierMySqlClient::onExecuteResult(quint64 requestId, const QString &json)
{
    (void)requestId;
    setBusy(false);
    if (json.isEmpty()) {
        m_lastError = QStringLiteral("execute failed (null result)");
        m_resultModel->clearAll();
        emit resultChanged();
        return;
    }
    ingestExecuteJson(json);
}

void PierMySqlClient::ingestExecuteJson(const QString &json)
{
    QJsonParseError parseErr {};
    const QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8(), &parseErr);
    if (parseErr.error != QJsonParseError::NoError || !doc.isObject()) {
        qWarning() << "PierMySqlClient: malformed execute JSON:" << parseErr.errorString();
        return;
    }
    const QJsonObject obj = doc.object();

    // Pull metadata first so callers can distinguish
    // "success with 0 rows" from "error".
    m_lastError          = obj.value(QStringLiteral("error")).toString();
    m_lastAffectedRows   = static_cast<qint64>(obj.value(QStringLiteral("affected_rows")).toDouble());
    m_lastElapsedMs      = static_cast<qint64>(obj.value(QStringLiteral("elapsed_ms")).toDouble());
    m_lastTruncated      = obj.value(QStringLiteral("truncated")).toBool();

    // Columns.
    QStringList columns;
    const QJsonArray colArr = obj.value(QStringLiteral("columns")).toArray();
    for (const QJsonValue &v : colArr) {
        columns.append(v.toString());
    }

    // Rows: preserve NULL-ness via parallel bool vector so the
    // table model can render `null` distinctly from empty.
    std::vector<std::vector<QString>> rows;
    std::vector<std::vector<bool>>    nulls;
    const QJsonArray rowsArr = obj.value(QStringLiteral("rows")).toArray();
    rows.reserve(static_cast<size_t>(rowsArr.size()));
    nulls.reserve(static_cast<size_t>(rowsArr.size()));
    for (const QJsonValue &rowVal : rowsArr) {
        if (!rowVal.isArray()) continue;
        const QJsonArray cells = rowVal.toArray();
        std::vector<QString> rowCells;
        std::vector<bool>    rowNulls;
        rowCells.reserve(static_cast<size_t>(cells.size()));
        rowNulls.reserve(static_cast<size_t>(cells.size()));
        for (const QJsonValue &cell : cells) {
            if (cell.isNull()) {
                rowCells.emplace_back();
                rowNulls.push_back(true);
            } else {
                rowCells.push_back(cell.toString());
                rowNulls.push_back(false);
            }
        }
        rows.push_back(std::move(rowCells));
        nulls.push_back(std::move(rowNulls));
    }

    m_resultModel->resetWith(columns, rows, nulls);
    emit resultChanged();
}

void PierMySqlClient::refreshDatabases()
{
    if (!m_handle) return;
    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);

    ::PierMysql *h = m_handle;
    QPointer<PierMySqlClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;

    auto worker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId, h
    ]() mutable {
        char *json = pier_mysql_list_databases(h);
        QString jsonStr;
        if (json) {
            jsonStr = QString::fromUtf8(json);
            pier_mysql_free_string(json);
        }
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onDatabasesResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(QString, jsonStr));
    });
    m_workers.push_back(std::move(worker));
}

void PierMySqlClient::onDatabasesResult(quint64 requestId, const QString &json)
{
    (void)requestId;
    setBusy(false);
    if (json.isEmpty()) return;
    ingestDatabasesJson(json);
}

void PierMySqlClient::ingestDatabasesJson(const QString &json)
{
    QJsonParseError parseErr {};
    const QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8(), &parseErr);
    if (parseErr.error != QJsonParseError::NoError || !doc.isArray()) {
        qWarning() << "PierMySqlClient: malformed databases JSON:" << parseErr.errorString();
        return;
    }
    QStringList dbs;
    const QJsonArray arr = doc.array();
    dbs.reserve(arr.size());
    for (const QJsonValue &v : arr) {
        dbs.append(v.toString());
    }
    m_databases = std::move(dbs);
    emit databasesChanged();
}

void PierMySqlClient::refreshTables(const QString &database)
{
    if (!m_handle) return;
    if (database.isEmpty()) {
        if (!m_tables.isEmpty()) {
            m_tables.clear();
            emit tablesChanged();
        }
        if (!m_columns.isEmpty()) {
            m_columns.clear();
            emit columnsChanged();
        }
        return;
    }
    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);

    std::string dbStd = database.toStdString();
    QString dbQt = database;
    ::PierMysql *h = m_handle;
    QPointer<PierMySqlClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;

    auto worker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId, h,
        dbStd = std::move(dbStd), dbQt
    ]() mutable {
        char *json = pier_mysql_list_tables(h, dbStd.c_str());
        QString jsonStr;
        if (json) {
            jsonStr = QString::fromUtf8(json);
            pier_mysql_free_string(json);
        }
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onTablesResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(QString, dbQt),
            Q_ARG(QString, jsonStr));
    });
    m_workers.push_back(std::move(worker));
}

void PierMySqlClient::onTablesResult(quint64 requestId, const QString &database, const QString &json)
{
    (void)requestId;
    (void)database;
    setBusy(false);
    if (json.isEmpty()) {
        if (!m_tables.isEmpty()) {
            m_tables.clear();
            emit tablesChanged();
        }
        return;
    }
    ingestTablesJson(json);
}

void PierMySqlClient::ingestTablesJson(const QString &json)
{
    QJsonParseError parseErr {};
    const QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8(), &parseErr);
    if (parseErr.error != QJsonParseError::NoError || !doc.isArray()) {
        qWarning() << "PierMySqlClient: malformed tables JSON:" << parseErr.errorString();
        return;
    }
    QStringList tables;
    const QJsonArray arr = doc.array();
    tables.reserve(arr.size());
    for (const QJsonValue &v : arr) {
        tables.append(v.toString());
    }
    m_tables = std::move(tables);
    emit tablesChanged();
}

void PierMySqlClient::refreshColumns(const QString &database, const QString &table)
{
    if (!m_handle) return;
    if (database.isEmpty() || table.isEmpty()) {
        if (!m_columns.isEmpty()) {
            m_columns.clear();
            emit columnsChanged();
        }
        return;
    }

    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);

    std::string dbStd = database.toStdString();
    std::string tableStd = table.toStdString();
    QString dbQt = database;
    QString tableQt = table;
    ::PierMysql *h = m_handle;
    QPointer<PierMySqlClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;

    auto worker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId, h,
        dbStd = std::move(dbStd), tableStd = std::move(tableStd), dbQt, tableQt
    ]() mutable {
        char *json = pier_mysql_list_columns(h, dbStd.c_str(), tableStd.c_str());
        QString jsonStr;
        if (json) {
            jsonStr = QString::fromUtf8(json);
            pier_mysql_free_string(json);
        }
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onColumnsResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(QString, dbQt),
            Q_ARG(QString, tableQt),
            Q_ARG(QString, jsonStr));
    });
    m_workers.push_back(std::move(worker));
}

void PierMySqlClient::onColumnsResult(
    quint64 requestId,
    const QString &database,
    const QString &table,
    const QString &json)
{
    (void)requestId;
    (void)database;
    (void)table;
    setBusy(false);
    if (json.isEmpty()) {
        if (!m_columns.isEmpty()) {
            m_columns.clear();
            emit columnsChanged();
        }
        return;
    }
    ingestColumnsJson(json);
}

void PierMySqlClient::ingestColumnsJson(const QString &json)
{
    QJsonParseError parseErr {};
    const QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8(), &parseErr);
    if (parseErr.error != QJsonParseError::NoError || !doc.isArray()) {
        qWarning() << "PierMySqlClient: malformed columns JSON:" << parseErr.errorString();
        return;
    }
    QVariantList columns;
    const QJsonArray arr = doc.array();
    columns.reserve(arr.size());
    for (const QJsonValue &v : arr) {
        if (!v.isObject()) continue;
        const QJsonObject obj = v.toObject();
        QVariantMap column;
        column.insert(QStringLiteral("name"), obj.value(QStringLiteral("name")).toString());
        column.insert(QStringLiteral("type"), obj.value(QStringLiteral("column_type")).toString());
        column.insert(QStringLiteral("nullable"), obj.value(QStringLiteral("nullable")).toBool());
        column.insert(QStringLiteral("key"), obj.value(QStringLiteral("key")).toString());
        column.insert(QStringLiteral("defaultValue"), obj.value(QStringLiteral("default_value")).toVariant());
        column.insert(QStringLiteral("extra"), obj.value(QStringLiteral("extra")).toString());
        columns.push_back(column);
    }
    m_columns = std::move(columns);
    emit columnsChanged();
}

void PierMySqlClient::stop()
{
    if (m_cancelFlag) {
        m_cancelFlag->store(true);
    }
    ++m_nextRequestId;
    if (m_handle) {
        ::PierMysql *h = m_handle;
        m_handle = nullptr;
        pier_mysql_free(h);
    }
    m_resultModel->clearAll();
    if (!m_databases.isEmpty()) {
        m_databases.clear();
        emit databasesChanged();
    }
    if (!m_tables.isEmpty()) {
        m_tables.clear();
        emit tablesChanged();
    }
    if (!m_columns.isEmpty()) {
        m_columns.clear();
        emit columnsChanged();
    }
    m_lastError.clear();
    m_lastAffectedRows = 0;
    m_lastElapsedMs = 0;
    m_lastTruncated = false;
    emit resultChanged();
    if (m_status != Idle) {
        m_errorMessage.clear();
        m_target.clear();
        setStatus(Idle);
    }
    setBusy(false);
}
