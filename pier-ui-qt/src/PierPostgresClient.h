// PierPostgresClient — Qt-side PostgreSQL panel (M7a)
//
// Mirrors PierMySqlClient byte-for-byte in API shape: same
// Status enum, same result model (QAbstractTableModel with
// display + isNull roles), same connectTo / execute /
// refreshDatabases / refreshTables / refreshColumns slots.
// The QML side can reuse the same MySqlPanelView-style layout
// with just the backend QML element name swapped.

#pragma once

#include <QAbstractTableModel>
#include <QObject>
#include <QPointer>
#include <QString>
#include <QStringList>
#include <QVariantList>
#include <qqml.h>

#include <atomic>
#include <cstdint>
#include <memory>
#include <thread>
#include <vector>

struct PierPostgres;

/// Result grid model — identical to PierMySqlResultModel.
class PierPgResultModel : public QAbstractTableModel
{
    Q_OBJECT
    QML_NAMED_ELEMENT(PierPgResultModel)
    QML_UNCREATABLE("Created by PierPostgresClient")

public:
    enum Roles { DisplayRole = Qt::UserRole + 1, IsNullRole };

    explicit PierPgResultModel(QObject *parent = nullptr);

    int rowCount(const QModelIndex &parent = QModelIndex()) const override;
    int columnCount(const QModelIndex &parent = QModelIndex()) const override;
    QVariant data(const QModelIndex &index, int role = Qt::DisplayRole) const override;
    QVariant headerData(int section, Qt::Orientation orientation,
                        int role = Qt::DisplayRole) const override;
    QHash<int, QByteArray> roleNames() const override;

    void resetWith(const QStringList &columns,
                   const std::vector<std::vector<QString>> &rows,
                   const std::vector<std::vector<bool>> &nulls);
    void clearAll();

private:
    struct Cell { QString value; bool isNull = false; };
    QStringList m_columns;
    std::vector<std::vector<Cell>> m_rows;
};

class PierPostgresClient : public QObject
{
    Q_OBJECT
    QML_NAMED_ELEMENT(PierPostgresClient)

public:
    enum Status { Idle = 0, Connecting = 1, Connected = 2, Failed = 3 };
    Q_ENUM(Status)

    Q_PROPERTY(Status status READ status NOTIFY statusChanged FINAL)
    Q_PROPERTY(QString errorMessage READ errorMessage NOTIFY statusChanged FINAL)
    Q_PROPERTY(QString target READ target NOTIFY statusChanged FINAL)
    Q_PROPERTY(bool busy READ busy NOTIFY busyChanged FINAL)
    Q_PROPERTY(QStringList databases READ databases NOTIFY databasesChanged FINAL)
    Q_PROPERTY(QStringList tables READ tables NOTIFY tablesChanged FINAL)
    Q_PROPERTY(QVariantList columns READ columns NOTIFY columnsChanged FINAL)
    Q_PROPERTY(QString lastError READ lastError NOTIFY resultChanged FINAL)
    Q_PROPERTY(qint64 lastAffectedRows READ lastAffectedRows NOTIFY resultChanged FINAL)
    Q_PROPERTY(qint64 lastElapsedMs READ lastElapsedMs NOTIFY resultChanged FINAL)
    Q_PROPERTY(bool lastTruncated READ lastTruncated NOTIFY resultChanged FINAL)
    Q_PROPERTY(int resultRowCount READ resultRowCount NOTIFY resultChanged FINAL)
    Q_PROPERTY(int resultColumnCount READ resultColumnCount NOTIFY resultChanged FINAL)
    Q_PROPERTY(PierPgResultModel *resultModel READ resultModel CONSTANT FINAL)

    explicit PierPostgresClient(QObject *parent = nullptr);
    ~PierPostgresClient() override;

    Status status() const { return m_status; }
    QString errorMessage() const { return m_errorMessage; }
    QString target() const { return m_target; }
    bool busy() const { return m_busy; }
    QStringList databases() const { return m_databases; }
    QStringList tables() const { return m_tables; }
    QVariantList columns() const { return m_columns; }
    QString lastError() const { return m_lastError; }
    qint64 lastAffectedRows() const { return m_lastAffectedRows; }
    qint64 lastElapsedMs() const { return m_lastElapsedMs; }
    bool lastTruncated() const { return m_lastTruncated; }
    int resultRowCount() const { return m_resultModel ? m_resultModel->rowCount() : 0; }
    int resultColumnCount() const { return m_resultModel ? m_resultModel->columnCount() : 0; }
    PierPgResultModel *resultModel() const { return m_resultModel; }

public slots:
    bool connectTo(const QString &host, int port,
                   const QString &user, const QString &password,
                   const QString &database);
    void execute(const QString &sql);
    void refreshDatabases();
    void refreshTables(const QString &schema);
    void refreshColumns(const QString &schema, const QString &table);
    void stop();

signals:
    void statusChanged();
    void busyChanged();
    void databasesChanged();
    void tablesChanged();
    void columnsChanged();
    void resultChanged();

private slots:
    void onConnectResult(quint64 requestId, void *handle, const QString &error);
    void onExecuteResult(quint64 requestId, const QString &json);
    void onDatabasesResult(quint64 requestId, const QString &json);
    void onTablesResult(quint64 requestId, const QString &json);
    void onColumnsResult(quint64 requestId, const QString &json);

private:
    void setStatus(Status s);
    void setBusy(bool b);
    void ingestExecuteJson(const QString &json);
    void ingestDatabasesJson(const QString &json);
    void ingestTablesJson(const QString &json);
    void ingestColumnsJson(const QString &json);

    ::PierPostgres *m_handle = nullptr;
    Status m_status = Idle;
    QString m_errorMessage;
    QString m_target;
    bool m_busy = false;
    QStringList m_databases;
    QStringList m_tables;
    QVariantList m_columns;
    QString m_lastError;
    qint64 m_lastAffectedRows = 0;
    qint64 m_lastElapsedMs = 0;
    bool m_lastTruncated = false;
    PierPgResultModel *m_resultModel = nullptr;
    quint64 m_nextRequestId = 0;
    std::shared_ptr<std::atomic<bool>> m_cancelFlag;
    std::vector<std::unique_ptr<std::thread>> m_workers;
};
