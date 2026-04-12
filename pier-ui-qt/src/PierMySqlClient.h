// ─────────────────────────────────────────────────────────
// PierMySqlClient — Qt-side MySQL panel
// ─────────────────────────────────────────────────────────
//
// Two QObjects in one file:
//
//   * PierMySqlResultModel — QAbstractTableModel that backs
//     the result grid (columns + rows with NULL cells). It
//     owns no connection state; it's populated from outside
//     whenever an execute() call returns.
//
//   * PierMySqlClientBackend — the panel's session object.
//     Wraps an opaque PierMysql handle, exposes connectTo /
//     execute / refresh slots, and hands every query result
//     to its PierMySqlResultModel child.
//
// Threading
// ─────────
//   Every pier_mysql_* call is blocking. We dispatch each
//   request on a dedicated std::thread per in-flight op and
//   post the result back via QMetaObject::invokeMethod, same
//   pattern as every other session-based QObject in the
//   app. A shared cancel flag + monotonic request id drops
//   stale deliveries if stop() races with an in-flight call.

#pragma once

#include <QAbstractTableModel>
#include <QObject>
#include <QPointer>
#include <QString>
#include <QStringList>
#include <qqml.h>

#include <atomic>
#include <cstdint>
#include <memory>
#include <thread>
#include <vector>

// Forward-declare the opaque Rust handle.
struct PierMysql;

/// Table model that backs the result grid in MySqlPanelView.
/// Cells are QVariant holding QString for value or
/// QVariant() (invalid) for NULL — this lets QML render
/// `null` distinctly without a separate role.
class PierMySqlResultModel : public QAbstractTableModel
{
    Q_OBJECT
    QML_NAMED_ELEMENT(PierMySqlResultModel)
    QML_UNCREATABLE("Created by PierMySqlClient")

public:
    enum Roles {
        DisplayRole = Qt::UserRole + 1,
        IsNullRole
    };

    explicit PierMySqlResultModel(QObject *parent = nullptr);

    int rowCount(const QModelIndex &parent = QModelIndex()) const override;
    int columnCount(const QModelIndex &parent = QModelIndex()) const override;
    QVariant data(const QModelIndex &index, int role = Qt::DisplayRole) const override;
    QVariant headerData(int section, Qt::Orientation orientation,
                        int role = Qt::DisplayRole) const override;
    QHash<int, QByteArray> roleNames() const override;

    /// Reset the model with a new column set + rows.
    /// `rows[r][c]` is either a non-null QString (the cell
    /// value) or an empty QString paired with `nulls[r][c]`
    /// set to true. Kept as parallel vectors so we can use
    /// flat layouts per row without std::variant overhead.
    void resetWith(const QStringList &columns,
                   const std::vector<std::vector<QString>> &rows,
                   const std::vector<std::vector<bool>> &nulls);

    /// Drop everything and emit model reset.
    void clearAll();

private:
    struct Cell {
        QString value;
        bool    isNull = false;
    };

    QStringList m_columns;
    std::vector<std::vector<Cell>> m_rows;
};


/// Panel backend. One instance per open MySQL tab.
class PierMySqlClient : public QObject
{
    Q_OBJECT
    QML_NAMED_ELEMENT(PierMySqlClient)

public:
    enum Status {
        Idle = 0,
        Connecting = 1,
        Connected = 2,
        Failed = 3
    };
    Q_ENUM(Status)

    Q_PROPERTY(Status status READ status NOTIFY statusChanged FINAL)
    Q_PROPERTY(QString errorMessage READ errorMessage NOTIFY statusChanged FINAL)
    Q_PROPERTY(QString target READ target NOTIFY statusChanged FINAL)
    Q_PROPERTY(bool busy READ busy NOTIFY busyChanged FINAL)

    // List of databases on the server (filtered — internal
    // schemas stripped). Populated after connect.
    Q_PROPERTY(QStringList databases READ databases NOTIFY databasesChanged FINAL)
    Q_PROPERTY(QStringList tables READ tables NOTIFY tablesChanged FINAL)

    // Last execute() result metadata.
    Q_PROPERTY(QString lastError READ lastError NOTIFY resultChanged FINAL)
    Q_PROPERTY(qint64 lastAffectedRows READ lastAffectedRows NOTIFY resultChanged FINAL)
    Q_PROPERTY(qint64 lastElapsedMs READ lastElapsedMs NOTIFY resultChanged FINAL)
    Q_PROPERTY(bool lastTruncated READ lastTruncated NOTIFY resultChanged FINAL)
    Q_PROPERTY(int resultRowCount READ resultRowCount NOTIFY resultChanged FINAL)
    Q_PROPERTY(int resultColumnCount READ resultColumnCount NOTIFY resultChanged FINAL)

    // The QAbstractTableModel driving the grid.
    Q_PROPERTY(PierMySqlResultModel *resultModel READ resultModel CONSTANT FINAL)

    explicit PierMySqlClient(QObject *parent = nullptr);
    ~PierMySqlClient() override;

    PierMySqlClient(const PierMySqlClient &) = delete;
    PierMySqlClient &operator=(const PierMySqlClient &) = delete;

    Status status() const { return m_status; }
    QString errorMessage() const { return m_errorMessage; }
    QString target() const { return m_target; }
    bool busy() const { return m_busy; }
    QStringList databases() const { return m_databases; }
    QStringList tables() const { return m_tables; }
    QString lastError() const { return m_lastError; }
    qint64 lastAffectedRows() const { return m_lastAffectedRows; }
    qint64 lastElapsedMs() const { return m_lastElapsedMs; }
    bool lastTruncated() const { return m_lastTruncated; }
    int resultRowCount() const { return m_resultModel ? m_resultModel->rowCount() : 0; }
    int resultColumnCount() const { return m_resultModel ? m_resultModel->columnCount() : 0; }
    PierMySqlResultModel *resultModel() const { return m_resultModel; }

public slots:
    /// Open a connection. `database` may be empty.
    bool connectTo(const QString &host, int port,
                   const QString &user, const QString &password,
                   const QString &database);

    /// Run a SQL statement. Result goes into resultModel +
    /// last* properties.
    void execute(const QString &sql);

    /// `SHOW DATABASES` — refreshes the `databases` property.
    void refreshDatabases();
    /// `SHOW TABLES FROM <database>` — refreshes the `tables`
    /// property. Empty database clears the list.
    void refreshTables(const QString &database);

    /// Tear down. Closes the handle and cancels in-flight
    /// ops. Safe to call multiple times.
    void stop();

signals:
    void statusChanged();
    void busyChanged();
    void databasesChanged();
    void tablesChanged();
    void resultChanged();

private slots:
    void onConnectResult(quint64 requestId, void *handle, const QString &error);
    void onExecuteResult(quint64 requestId, const QString &json);
    void onDatabasesResult(quint64 requestId, const QString &json);
    void onTablesResult(quint64 requestId, const QString &database, const QString &json);

private:
    void setStatus(Status s);
    void setBusy(bool b);
    void ingestExecuteJson(const QString &json);
    void ingestDatabasesJson(const QString &json);
    void ingestTablesJson(const QString &json);

    ::PierMysql *m_handle = nullptr;

    Status m_status = Status::Idle;
    QString m_errorMessage;
    QString m_target;
    bool m_busy = false;

    QStringList m_databases;
    QStringList m_tables;

    QString m_lastError;
    qint64 m_lastAffectedRows = 0;
    qint64 m_lastElapsedMs = 0;
    bool m_lastTruncated = false;

    PierMySqlResultModel *m_resultModel = nullptr;

    quint64 m_nextRequestId = 0;
    std::shared_ptr<std::atomic<bool>> m_cancelFlag;
    std::vector<std::unique_ptr<std::thread>> m_workers;
};
