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
#include <QVariantList>
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
    Q_PROPERTY(bool inspectBusy READ inspectBusy NOTIFY inspectStateChanged FINAL)
    Q_PROPERTY(QString inspectError READ inspectError NOTIFY inspectStateChanged FINAL)
    Q_PROPERTY(QString inspectTarget READ inspectTarget NOTIFY inspectStateChanged FINAL)
    Q_PROPERTY(QString inspectJson READ inspectJson NOTIFY inspectStateChanged FINAL)

    // Images / Volumes / Networks
    Q_PROPERTY(QVariantList images READ images NOTIFY imagesChanged FINAL)
    Q_PROPERTY(QVariantList volumes READ volumes NOTIFY volumesChanged FINAL)
    Q_PROPERTY(QVariantList networks READ networks NOTIFY networksChanged FINAL)

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
    bool inspectBusy() const { return m_inspectBusy; }
    QString inspectError() const { return m_inspectError; }
    QString inspectTarget() const { return m_inspectTarget; }
    QString inspectJson() const { return m_inspectJson; }
    QVariantList images() const { return m_images; }
    QVariantList volumes() const { return m_volumes; }
    QVariantList networks() const { return m_networks; }

    void setShowStopped(bool v);

public slots:
    /// Open the SSH session behind the panel.
    bool connectTo(const QString &host, int port, const QString &user,
                   int authKind, const QString &secret, const QString &extra);

    /// Reuse an existing shared SSH session (no extra handshake).
    bool connectToSession(QObject *sessionHandle);

    /// Connect to the local Docker daemon (no SSH needed).
    bool connectLocal();

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
    void inspect(const QString &id);
    void clearInspect();

    /// Image/Volume/Network management
    void refreshImages();
    void removeImage(const QString &id, bool force);
    void refreshVolumes();
    void removeVolume(const QString &name);
    void refreshNetworks();
    void removeNetwork(const QString &name);

    /// Shut down.
    void stop();

signals:
    void statusChanged();
    void busyChanged();
    void showStoppedChanged();
    void containerCountChanged();
    void inspectStateChanged();
    void imagesChanged();
    void volumesChanged();
    void networksChanged();
    /// Fired once an action slot completes so the UI can
    /// show a transient toast without binding to the busy
    /// property transitions.
    void actionFinished(bool ok, const QString &message);

private slots:
    void onConnectResult(quint64 requestId, void *handle, const QString &error);
    void onListResult(quint64 requestId, const QString &json, const QString &error);
    void onActionResult(quint64 requestId, bool ok, const QString &message);
    void onInspectResult(quint64 requestId, const QString &id, const QString &json, const QString &error);
    void onImagesResult(quint64 requestId, const QString &json);
    void onVolumesResult(quint64 requestId, const QString &json);
    void onNetworksResult(quint64 requestId, const QString &json);

private:
    void setStatus(Status s);
    void setBusy(bool b);
    void setInspectBusy(bool b);
    void ingestListJson(const QString &json);
    void spawnList(quint64 requestId);
    void spawnAction(quint64 requestId,
                     const QString &verb,
                     const QString &id,
                     bool force);
    static QString formatInspectJson(const QString &json);

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
    bool m_localMode = false;
    std::vector<Row> m_rows;

    Status m_status = Status::Idle;
    QString m_errorMessage;
    QString m_target;
    bool m_busy = false;
    bool m_showStopped = true;
    bool m_inspectBusy = false;
    QString m_inspectError;
    QString m_inspectTarget;
    QString m_inspectJson;

    QVariantList m_images;
    QVariantList m_volumes;
    QVariantList m_networks;

    quint64 m_nextRequestId = 0;
    quint64 m_nextInspectRequestId = 0;
    std::shared_ptr<std::atomic<bool>> m_cancelFlag;
    std::vector<std::unique_ptr<std::thread>> m_workers;
};
