// ─────────────────────────────────────────────────────────
// PierServiceDetector — async remote service discovery
// ─────────────────────────────────────────────────────────
//
// Probes a remote host for known services (MySQL / Redis /
// PostgreSQL / Docker) via `pier_services_detect`. One
// instance per terminal tab; the QML side calls
// `detect(host, port, user, authKind, secret, extra)` after
// the SSH session has successfully connected, and binds a
// strip of status pills to the `services` model.
//
// Threading: detection runs on a dedicated std::thread. The
// result is posted back to the main thread via queued
// invoke, identical pattern to PierSftpBrowser and
// PierTerminalSession. A shared cancel flag + bumped request
// id drops stale deliveries if the tab is closed mid-detect.

#pragma once

#include <QAbstractListModel>
#include <QObject>
#include <QPointer>
#include <QString>
#include <qqml.h>

#include <atomic>
#include <cstdint>
#include <memory>
#include <thread>
#include <vector>

class PierServiceDetector : public QAbstractListModel
{
    Q_OBJECT
    QML_NAMED_ELEMENT(PierServiceDetector)

public:
    enum Roles {
        NameRole = Qt::UserRole + 1,
        VersionRole,
        StatusRole,    // "running" | "stopped" | "installed"
        PortRole
    };

    enum State {
        Idle = 0,
        Running = 1,   // detection in progress
        Done = 2,      // finished successfully; model is populated
        Failed = 3
    };
    Q_ENUM(State)

    Q_PROPERTY(State state READ state NOTIFY stateChanged FINAL)
    Q_PROPERTY(QString errorMessage READ errorMessage NOTIFY stateChanged FINAL)
    Q_PROPERTY(int count READ count NOTIFY countChanged FINAL)

    explicit PierServiceDetector(QObject *parent = nullptr);
    ~PierServiceDetector() override;

    PierServiceDetector(const PierServiceDetector &) = delete;
    PierServiceDetector &operator=(const PierServiceDetector &) = delete;

    int rowCount(const QModelIndex &parent = QModelIndex()) const override;
    QVariant data(const QModelIndex &index, int role = Qt::DisplayRole) const override;
    QHash<int, QByteArray> roleNames() const override;

    State state() const { return m_state; }
    QString errorMessage() const { return m_errorMessage; }
    int count() const { return static_cast<int>(m_entries.size()); }

public slots:
    // Kick off detection. Returns true if the worker thread
    // was successfully scheduled. Watch `state` for the
    // transition to Done or Failed.
    //
    // `authKind` + `secret` + `extra` follow the table in
    // pier_services.h.
    bool detect(const QString &host, int port, const QString &user,
                int authKind, const QString &secret, const QString &extra);

    // Cancel any in-flight detection and clear the model.
    // Any result that arrives after cancel() is silently
    // dropped.
    void cancel();

signals:
    void stateChanged();
    void countChanged();

private slots:
    // Runs on the main thread, called via queued connection
    // from the worker.
    void onDetectResult(quint64 requestId, const QString &json, const QString &error);

private:
    struct Entry {
        QString name;
        QString version;
        QString status;
        int     port = 0;
    };

    void setState(State s);
    void ingestJson(const QString &json);

    std::vector<Entry> m_entries;
    State   m_state = State::Idle;
    QString m_errorMessage;

    quint64 m_nextRequestId = 0;
    std::shared_ptr<std::atomic<bool>> m_cancelFlag;
    std::unique_ptr<std::thread> m_worker;
};
