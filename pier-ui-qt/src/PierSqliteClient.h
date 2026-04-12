#pragma once

#include <QObject>
#include <QPointer>
#include <QString>
#include <QStringList>
#include <QVariantList>
#include <qqml.h>

#include <atomic>
#include <memory>
#include <thread>
#include <vector>

struct PierSqlite;

class PierSqliteClient : public QObject
{
    Q_OBJECT
    QML_NAMED_ELEMENT(PierSqliteClient)

public:
    enum Status { Idle = 0, Loading = 1, Ready = 2, Failed = 3 };
    Q_ENUM(Status)

    Q_PROPERTY(Status status READ status NOTIFY statusChanged FINAL)
    Q_PROPERTY(QString errorMessage READ errorMessage NOTIFY statusChanged FINAL)
    Q_PROPERTY(QString dbPath READ dbPath NOTIFY dbChanged FINAL)
    Q_PROPERTY(bool busy READ busy NOTIFY busyChanged FINAL)
    Q_PROPERTY(QStringList tables READ tables NOTIFY tablesChanged FINAL)
    Q_PROPERTY(QVariantList columns READ columns NOTIFY columnsChanged FINAL)
    Q_PROPERTY(QVariantList resultColumns READ resultColumns NOTIFY resultChanged FINAL)
    Q_PROPERTY(QVariantList resultRows READ resultRows NOTIFY resultChanged FINAL)
    Q_PROPERTY(qint64 lastElapsedMs READ lastElapsedMs NOTIFY resultChanged FINAL)
    Q_PROPERTY(QString lastError READ lastError NOTIFY resultChanged FINAL)

    explicit PierSqliteClient(QObject *parent = nullptr);
    ~PierSqliteClient() override;

    Status status() const { return m_status; }
    QString errorMessage() const { return m_errorMessage; }
    QString dbPath() const { return m_dbPath; }
    bool busy() const { return m_busy; }
    QStringList tables() const { return m_tables; }
    QVariantList columns() const { return m_columns; }
    QVariantList resultColumns() const { return m_resultColumns; }
    QVariantList resultRows() const { return m_resultRows; }
    qint64 lastElapsedMs() const { return m_lastElapsedMs; }
    QString lastError() const { return m_lastError; }

public slots:
    void open(const QString &path);
    void refreshTables();
    void loadColumns(const QString &table);
    void execute(const QString &sql);
    void close();

signals:
    void statusChanged();
    void dbChanged();
    void busyChanged();
    void tablesChanged();
    void columnsChanged();
    void resultChanged();

private slots:
    void onOpenResult(quint64 id, void *handle, const QString &error);
    void onTablesResult(quint64 id, const QString &json);
    void onColumnsResult(quint64 id, const QString &json);
    void onExecuteResult(quint64 id, const QString &json);

private:
    void setStatus(Status s);
    void setBusy(bool b);

    ::PierSqlite *m_handle = nullptr;
    Status m_status = Idle;
    QString m_errorMessage;
    QString m_dbPath;
    bool m_busy = false;
    QStringList m_tables;
    QVariantList m_columns;
    QVariantList m_resultColumns;
    QVariantList m_resultRows;
    qint64 m_lastElapsedMs = 0;
    QString m_lastError;
    quint64 m_nextRequestId = 0;
    std::shared_ptr<std::atomic<bool>> m_cancelFlag;
    std::vector<std::unique_ptr<std::thread>> m_workers;
};
