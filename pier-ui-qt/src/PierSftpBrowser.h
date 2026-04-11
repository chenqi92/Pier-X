// ─────────────────────────────────────────────────────────
// PierSftpBrowser — Qt-side SFTP file browser
// ─────────────────────────────────────────────────────────
//
// Wraps an opaque PierSftp handle and exposes a QAbstractListModel
// of the current directory plus the navigation / mutation slots
// QML needs for a file browser panel.
//
// Threading
// ─────────
//   Every pier_sftp_* call is blocking. We run them on a
//   dedicated std::thread per in-flight request, same pattern
//   as PierTerminalSession's async SSH connect. The worker
//   captures a QPointer<self> + a request id; when the call
//   returns it posts onListResult / onOpResult back to the
//   main thread via QMetaObject::invokeMethod(QueuedConnection).
//
// Lifecycle
// ─────────
//   * Instantiated in QML. Idle until `connectTo(...)` is
//     called (async — produces Connected or Failed).
//   * After Connected, `listDir(path)` loads a directory and
//     populates the model.
//   * Destruction cancels any in-flight op via
//     shared cancel flag and detaches the thread.

#pragma once

#include <QAbstractListModel>
#include <QObject>
#include <QPointer>
#include <QString>
#include <QVariantMap>
#include <qqml.h>

#include <atomic>
#include <cstdint>
#include <memory>
#include <thread>
#include <vector>

// Forward-declare the opaque Rust handle.
struct PierSftp;

class PierSftpBrowser : public QAbstractListModel
{
    Q_OBJECT
    QML_NAMED_ELEMENT(PierSftpBrowser)

public:
    enum Status {
        Idle = 0,
        Connecting = 1,
        Connected = 2,
        Failed = 3
    };
    Q_ENUM(Status)

    enum Roles {
        NameRole = Qt::UserRole + 1,
        PathRole,
        IsDirRole,
        IsLinkRole,
        SizeRole,
        ModifiedRole
    };

    Q_PROPERTY(Status status READ status NOTIFY statusChanged FINAL)
    Q_PROPERTY(QString errorMessage READ errorMessage NOTIFY statusChanged FINAL)
    Q_PROPERTY(QString target READ target NOTIFY statusChanged FINAL)
    Q_PROPERTY(QString currentPath READ currentPath NOTIFY currentPathChanged FINAL)
    Q_PROPERTY(bool busy READ busy NOTIFY busyChanged FINAL)

    explicit PierSftpBrowser(QObject *parent = nullptr);
    ~PierSftpBrowser() override;

    PierSftpBrowser(const PierSftpBrowser &) = delete;
    PierSftpBrowser &operator=(const PierSftpBrowser &) = delete;

    // QAbstractListModel interface.
    int rowCount(const QModelIndex &parent = QModelIndex()) const override;
    QVariant data(const QModelIndex &index, int role = Qt::DisplayRole) const override;
    QHash<int, QByteArray> roleNames() const override;

    Status status() const { return m_status; }
    QString errorMessage() const { return m_errorMessage; }
    QString target() const { return m_target; }
    QString currentPath() const { return m_currentPath; }
    bool busy() const { return m_busy; }

public slots:
    // Dial the remote SFTP server and open a session. Returns
    // true if the worker thread was successfully scheduled;
    // watch `status` for Connected / Failed.
    //
    // `authKind` matches the C header constants
    // (0=password, 1=credential, 2=key, 3=agent).
    // `secret` and `extra` follow the auth_kind table in
    // pier_sftp.h.
    bool connectTo(const QString &host, int port, const QString &user,
                   int authKind, const QString &secret, const QString &extra);

    // Navigate to `path` and refresh the listing. Async —
    // watch `busy` + `currentPath` for completion. On error
    // the status transitions to Failed with an errorMessage.
    void listDir(const QString &path);

    // Go up one directory. Computes parent of currentPath
    // and calls listDir(). No-op at root.
    void navigateUp();

    // Refresh the current listing (re-fetches from the server).
    void refresh();

    // Basic mutations. All async, watch `busy`.
    void mkdir(const QString &path);
    void removeFile(const QString &path);
    void removeDir(const QString &path);
    void rename(const QString &from, const QString &to);

    // Shut down. Closes the handle and cancels any in-flight
    // operation. Safe to call multiple times.
    void stop();

signals:
    void statusChanged();
    void currentPathChanged();
    void busyChanged();
    /// Fired once an operation (mkdir/remove/rename) completes
    /// so the QML side can refresh without relying on property
    /// change signals.
    void operationFinished(bool ok, const QString &message);

private slots:
    // Delivery slots called via QueuedConnection from worker threads.
    void onConnectResult(quint64 requestId, void *handle, const QString &error, const QString &canonicalCwd);
    void onListResult(quint64 requestId, const QString &path, const QString &jsonEntries, const QString &error);
    void onOperationResult(quint64 requestId, bool ok, const QString &message);

private:
    void setStatus(Status s);
    void setBusy(bool b);
    void ingestListJson(const QString &json);
    void spawnList(quint64 requestId, const QString &path);

    // POD for model rows.
    struct Entry {
        QString name;
        QString path;
        bool    isDir = false;
        bool    isLink = false;
        qint64  size = 0;
        qint64  modified = 0; // seconds since epoch, 0 if unknown
    };

    PierSftp *m_handle = nullptr;
    std::vector<Entry> m_entries;

    Status  m_status = Status::Idle;
    QString m_errorMessage;
    QString m_target;
    QString m_currentPath;
    bool    m_busy = false;

    quint64 m_nextRequestId = 0;
    // Shared cancel flag so in-flight worker threads know to
    // discard their results if stop() fires before they return.
    std::shared_ptr<std::atomic<bool>> m_cancelFlag;
    // Keep detached thread handles alive until drop — we don't
    // join, but we want the unique_ptr destruction to happen
    // at a well-defined point rather than in a signal slot.
    std::vector<std::unique_ptr<std::thread>> m_workers;
};
