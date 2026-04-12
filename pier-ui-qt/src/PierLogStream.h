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
#include <QProcess>
#include <QRegularExpression>
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
        TextRole,
        LevelRole
    };

    enum Level {
        UnknownLevel = 0,
        DebugLevel = 1,
        InfoLevel = 2,
        WarnLevel = 3,
        ErrorLevel = 4,
        FatalLevel = 5
    };
    Q_ENUM(Level)

    Q_PROPERTY(Status status READ status NOTIFY statusChanged FINAL)
    Q_PROPERTY(QString errorMessage READ errorMessage NOTIFY statusChanged FINAL)
    Q_PROPERTY(QString target READ target NOTIFY statusChanged FINAL)
    Q_PROPERTY(QString command READ command WRITE setCommand NOTIFY commandChanged FINAL)
    Q_PROPERTY(int exitCode READ exitCode NOTIFY statusChanged FINAL)
    Q_PROPERTY(int lineCount READ lineCount NOTIFY lineCountChanged FINAL)
    Q_PROPERTY(int totalLineCount READ totalLineCount NOTIFY totalLineCountChanged FINAL)
    Q_PROPERTY(bool alive READ alive NOTIFY statusChanged FINAL)
    Q_PROPERTY(QString filterText READ filterText WRITE setFilterText NOTIFY filterStateChanged FINAL)
    Q_PROPERTY(bool regexMode READ regexMode WRITE setRegexMode NOTIFY filterStateChanged FINAL)
    Q_PROPERTY(QString regexError READ regexError NOTIFY filterStateChanged FINAL)
    Q_PROPERTY(bool debugEnabled READ debugEnabled WRITE setDebugEnabled NOTIFY levelFiltersChanged FINAL)
    Q_PROPERTY(bool infoEnabled READ infoEnabled WRITE setInfoEnabled NOTIFY levelFiltersChanged FINAL)
    Q_PROPERTY(bool warnEnabled READ warnEnabled WRITE setWarnEnabled NOTIFY levelFiltersChanged FINAL)
    Q_PROPERTY(bool errorEnabled READ errorEnabled WRITE setErrorEnabled NOTIFY levelFiltersChanged FINAL)
    Q_PROPERTY(bool fatalEnabled READ fatalEnabled WRITE setFatalEnabled NOTIFY levelFiltersChanged FINAL)

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
    int lineCount() const { return static_cast<int>(m_visibleRows.size()); }
    int totalLineCount() const { return static_cast<int>(m_rows.size()); }
    bool alive() const { return m_status == Connected; }
    QString filterText() const { return m_filterText; }
    bool regexMode() const { return m_regexMode; }
    QString regexError() const { return m_regexError; }
    bool debugEnabled() const { return m_debugEnabled; }
    bool infoEnabled() const { return m_infoEnabled; }
    bool warnEnabled() const { return m_warnEnabled; }
    bool errorEnabled() const { return m_errorEnabled; }
    bool fatalEnabled() const { return m_fatalEnabled; }

    void setCommand(const QString &cmd);
    void setFilterText(const QString &text);
    void setRegexMode(bool enabled);
    void setDebugEnabled(bool enabled);
    void setInfoEnabled(bool enabled);
    void setWarnEnabled(bool enabled);
    void setErrorEnabled(bool enabled);
    void setFatalEnabled(bool enabled);

public slots:
    /// Dial the remote and start streaming `command`. Same
    /// auth-kind semantics as every other session-based API
    /// (0=password, 1=credential, 2=key, 3=agent).
    bool connectTo(const QString &host, int port, const QString &user,
                   int authKind, const QString &secret, const QString &extra,
                   const QString &command);
    bool connectToSession(QObject *sessionHandle, const QString &command);
    bool connectLocal(const QString &command);

    /// Drop every row currently in the model.
    void clear();

    /// Stop the remote process and close the handle. Safe to
    /// call multiple times.
    void stop();

signals:
    void statusChanged();
    void commandChanged();
    void lineCountChanged();
    void totalLineCountChanged();
    void filterStateChanged();
    void levelFiltersChanged();

private slots:
    void onConnectResult(quint64 requestId, void *handle, const QString &error);
    /// Fired on the Qt main thread by m_pollTimer. Drains every
    /// pending event from the backend handle and appends rows.
    void onPollTick();

private:
    struct Row {
        Kind kind;
        QString text;
        Level level = UnknownLevel;
    };

    void setStatus(Status s);
    void ingestEventsJson(const QString &json);
    void trimIfOverflow();
    void rebuildVisibleRows();
    void refreshFilterRegex();
    bool rowMatchesFilters(const Row &row) const;
    static Level detectLevel(Kind kind, const QString &text);

    ::PierLogStream *m_handle = nullptr;
    QProcess *m_localProcess = nullptr;
    std::deque<Row> m_rows;
    std::vector<int> m_visibleRows;

    Status m_status = Status::Idle;
    QString m_errorMessage;
    QString m_target;
    QString m_command;
    int m_exitCode = -1;

    QTimer m_pollTimer;

    QString m_filterText;
    bool m_regexMode = false;
    QString m_regexError;
    QRegularExpression m_filterRegex;
    bool m_debugEnabled = true;
    bool m_infoEnabled = true;
    bool m_warnEnabled = true;
    bool m_errorEnabled = true;
    bool m_fatalEnabled = true;

    quint64 m_nextRequestId = 0;
    std::shared_ptr<std::atomic<bool>> m_cancelFlag;
    std::vector<std::unique_ptr<std::thread>> m_workers;
};
