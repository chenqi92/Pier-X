// ─────────────────────────────────────────────────────────
// PierDockerClient — Qt-side Docker container panel
// ─────────────────────────────────────────────────────────
//
// Wraps an opaque PierDocker handle (see pier_docker.h) and
// exposes the list of remote containers as a QAbstractListModel.
//
// Threading
// ─────────
//   Every pier_docker_* function is blocking (each one runs a
//   fresh `docker <verb>` over SSH exec). We dispatch them on
//   a dedicated std::thread per in-flight request — same
//   pattern as PierSftpBrowser / PierRedisClient / PierLogStream.
//   Results are delivered back on the main thread via
//   QMetaObject::invokeMethod(QueuedConnection). A shared
//   cancel flag + monotonic request id drop stale deliveries
//   if stop() races with an in-flight op.
//
// Lifecycle
// ─────────
//   * Instantiated in QML with backend params set.
//   * connectTo(...) spawns the SSH session (async — watch
//     `status` for Connected / Failed).
//   * While Connected, refresh() pulls `docker ps`, action
//     slots (start/stop/restart/remove) run a verb then
//     auto-refresh on success.
//   * stop() tears down the handle. Safe to call multiple
//     times.
//
// Note: live logs for a single container don't live here —
// they flow through the Log viewer panel. The Main.qml
// delegate wires the row's "logs" button to open a new Log
// tab whose command is `docker logs -f --tail 500 <id>`.

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

// Forward-declare the opaque Rust handle.
struct PierDocker;

class PierDockerClientModel : public QAbstractListModel
{
    Q_OBJECT
    QML_NAMED_ELEMENT(PierDockerClient)

public:
    enum Status {
        Idle = 0,
        Connecting = 1,
        Connected = 2,
        Failed = 3
    };
    Q_ENUM(Status)

    enum Roles {
        IdRole = Qt::UserRole + 1,
        ImageRole,
        NamesRole,
        StatusTextRole,
        StateRole,
        IsRunningRole,
        CreatedRole,
        PortsRole
    };

    Q_PROPERTY(Status status READ status NOTIFY statusChanged FINAL)
    Q_PROPERTY(QString errorMessage READ errorMessage NOTIFY statusChanged FINAL)
    Q_PROPERTY(QString target READ target NOTIFY statusChanged FINAL)
    Q_PROPERTY(bool busy READ busy NOTIFY busyChanged FINAL)
    Q_PROPERTY(bool showStopped READ showStopped WRITE setShowStopped NOTIFY showStoppedChanged FINAL)
    Q_PROPERTY(int containerCount READ containerCount NOTIFY containerCountChanged FINAL)

    explicit PierDockerClientModel(QObject *parent = nullptr);
    ~PierDockerClientModel() override;

    PierDockerClientModel(const PierDockerClientModel &) = delete;
    PierDockerClientModel &operator=(const PierDockerClientModel &) = delete;

    int rowCount(const QModelIndex &parent = QModelIndex()) const override;
    QVariant data(const QModelIndex &index, int role = Qt::DisplayRole) const override;
    QHash<int, QByteArray> roleNames() const override;

    Status status() const { return m_status; }
    QString errorMessage() const { return m_errorMessage; }
    QString target() const { return m_target; }
    bool busy() const { return m_busy; }
    bool showStopped() const { return m_showStopped; }
    int containerCount() const { return static_cast<int>(m_rows.size()); }

    void setShowStopped(bool v);

public slots:
    /// Open the SSH session behind the panel. Same auth-kind
    /// table as every other session-based QObject here
    /// (0=password, 1=credential, 2=key, 3=agent).
    bool connectTo(const QString &host, int port, const QString &user,
                   int authKind, const QString &secret, const QString &extra);

    /// Re-run `docker ps` and replace the model contents.
    void refresh();

    /// Action slots. All async, all watch `busy`. On success
    /// they trigger a refresh() automatically so the UI
    /// reflects the new state without the QML side having to
    /// re-call.
    void start(const QString &id);
    void stopContainer(const QString &id);
    void restart(const QString &id);
    void remove(const QString &id, bool force);

    /// Shut down. Cancels any in-flight op and closes the
    /// handle. Safe to call more than once.
    void stop();

signals:
    void statusChanged();
    void busyChanged();
    void showStoppedChanged();
    void containerCountChanged();
    /// Fired once an action slot completes so the UI can
    /// show a transient toast without binding to the busy
    /// property transitions.
    void actionFinished(bool ok, const QString &message);

private slots:
    void onConnectResult(quint64 requestId, void *handle, const QString &error);
    void onListResult(quint64 requestId, const QString &json, const QString &error);
    void onActionResult(quint64 requestId, bool ok, const QString &message);

private:
    void setStatus(Status s);
    void setBusy(bool b);
    void ingestListJson(const QString &json);
    void spawnList(quint64 requestId);
    void spawnAction(quint64 requestId,
                     const QString &verb,
                     const QString &id,
                     bool force);

    struct Row {
        QString id;
        QString image;
        QString names;
        QString statusText;
        QString state;
        bool    isRunning = false;
        QString created;
        QString ports;
    };

    ::PierDocker *m_handle = nullptr;
    std::vector<Row> m_rows;

    Status m_status = Status::Idle;
    QString m_errorMessage;
    QString m_target;
    bool m_busy = false;
    bool m_showStopped = true;

    quint64 m_nextRequestId = 0;
    std::shared_ptr<std::atomic<bool>> m_cancelFlag;
    std::vector<std::unique_ptr<std::thread>> m_workers;
};
