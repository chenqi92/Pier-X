// ─────────────────────────────────────────────────────────
// PierLogStream — Qt-side streaming remote log viewer
// ─────────────────────────────────────────────────────────
//
// Wraps an opaque PierLogStream handle (see pier_log.h) and
// exposes its events as a QAbstractListModel. The model is
// append-only from the UI's perspective: rows are added as
// new lines arrive and the oldest rows are trimmed off the
// head once the buffer cap (see MAX_ROWS) is hit so a
// pathological log source can't blow the UI heap.
//
// Threading
// ─────────
//   pier_log_open is blocking. We run it on a dedicated
//   std::thread per connectTo call (same pattern as
//   PierSftpBrowser / PierTunnelHandle). Once the handle is
//   live the UI polls pier_log_drain on a Qt main-thread
//   QTimer — drain is a non-blocking try_recv loop so this is
//   always safe. The producer task in Rust runs on the shared
//   tokio runtime and never crosses back into Qt land.
//
// Lifecycle
// ─────────
//   * Instantiated in QML with backend params set.
//   * connectTo(...) spawns the SSH session + exec stream
//     (async — watch `status` for Connected / Failed).
//   * While Connected, every tick of the pollTimer drains
//     events and appends rows. The timer auto-stops when the
//     backend signals a non-alive handle + no more events.
//   * stop() flips the backend's stop flag and releases the
//     handle. Safe to call multiple times.

#pragma once

#include <QAbstractListModel>
#include <QObject>
#include <QPointer>
#include <QString>
#include <QTimer>
#include <qqml.h>

#include <atomic>
#include <cstdint>
#include <deque>
#include <memory>
#include <thread>
#include <vector>

// Forward-declare the opaque Rust handle.
struct PierLogStream;

class PierLogStreamModel : public QAbstractListModel
{
    Q_OBJECT
    QML_NAMED_ELEMENT(PierLogStream)

public:
    enum Status {
        Idle = 0,
        Connecting = 1,
        Connected = 2,
        Failed = 3,
        Finished = 4
    };
    Q_ENUM(Status)

    enum Kind {
        Stdout = 0,
        Stderr = 1,
        Exit = 2,
        ErrorKind = 3
    };
    Q_ENUM(Kind)

    enum Roles {
        KindRole = Qt::UserRole + 1,
        TextRole
    };

    Q_PROPERTY(Status status READ status NOTIFY statusChanged FINAL)
    Q_PROPERTY(QString errorMessage READ errorMessage NOTIFY statusChanged FINAL)
    Q_PROPERTY(QString target READ target NOTIFY statusChanged FINAL)
    Q_PROPERTY(QString command READ command WRITE setCommand NOTIFY commandChanged FINAL)
    Q_PROPERTY(int exitCode READ exitCode NOTIFY statusChanged FINAL)
    Q_PROPERTY(int lineCount READ lineCount NOTIFY lineCountChanged FINAL)
    Q_PROPERTY(bool alive READ alive NOTIFY statusChanged FINAL)

    /// Maximum number of rows retained before trimming.
    /// 5k is enough for a "what just happened" scroll but
    /// small enough that a runaway producer can't OOM the UI.
    static constexpr int MAX_ROWS = 5000;

    explicit PierLogStreamModel(QObject *parent = nullptr);
    ~PierLogStreamModel() override;

    PierLogStreamModel(const PierLogStreamModel &) = delete;
    PierLogStreamModel &operator=(const PierLogStreamModel &) = delete;

    // QAbstractListModel.
    int rowCount(const QModelIndex &parent = QModelIndex()) const override;
    QVariant data(const QModelIndex &index, int role = Qt::DisplayRole) const override;
    QHash<int, QByteArray> roleNames() const override;

    Status status() const { return m_status; }
    QString errorMessage() const { return m_errorMessage; }
    QString target() const { return m_target; }
    QString command() const { return m_command; }
    int exitCode() const { return m_exitCode; }
    int lineCount() const { return static_cast<int>(m_rows.size()); }
    bool alive() const { return m_status == Connected; }

    void setCommand(const QString &cmd);

public slots:
    /// Dial the remote and start streaming `command`. Same
    /// auth-kind semantics as every other session-based API
    /// (0=password, 1=credential, 2=key, 3=agent).
    bool connectTo(const QString &host, int port, const QString &user,
                   int authKind, const QString &secret, const QString &extra,
                   const QString &command);

    /// Drop every row currently in the model.
    void clear();

    /// Stop the remote process and close the handle. Safe to
    /// call multiple times.
    void stop();

signals:
    void statusChanged();
    void commandChanged();
    void lineCountChanged();

private slots:
    void onConnectResult(quint64 requestId, void *handle, const QString &error);
    /// Fired on the Qt main thread by m_pollTimer. Drains every
    /// pending event from the backend handle and appends rows.
    void onPollTick();

private:
    void setStatus(Status s);
    void ingestEventsJson(const QString &json);
    void trimIfOverflow();

    struct Row {
        Kind kind;
        QString text;
    };

    ::PierLogStream *m_handle = nullptr;
    std::deque<Row> m_rows;

    Status m_status = Status::Idle;
    QString m_errorMessage;
    QString m_target;
    QString m_command;
    int m_exitCode = -1;

    QTimer m_pollTimer;

    quint64 m_nextRequestId = 0;
    std::shared_ptr<std::atomic<bool>> m_cancelFlag;
    std::vector<std::unique_ptr<std::thread>> m_workers;
};
