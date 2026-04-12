#include "PierLogStream.h"

#include "pier_log.h"

#include <QByteArray>
#include <QDebug>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonValue>
#include <QMetaObject>

PierLogStreamModel::PierLogStreamModel(QObject *parent)
    : QAbstractListModel(parent)
{
    // 200 ms poll cadence: fast enough that scrolling feels
    // live, slow enough that a flood of log lines batches
    // naturally into one beginInsertRows/endInsertRows call.
    m_pollTimer.setInterval(200);
    m_pollTimer.setSingleShot(false);
    connect(&m_pollTimer, &QTimer::timeout, this, &PierLogStreamModel::onPollTick);
}

PierLogStreamModel::~PierLogStreamModel()
{
    stop();
    for (auto &t : m_workers) {
        if (t && t->joinable()) {
            t->detach();
        }
    }
}

int PierLogStreamModel::rowCount(const QModelIndex &parent) const
{
    if (parent.isValid()) return 0;
    return static_cast<int>(m_visibleRows.size());
}

QVariant PierLogStreamModel::data(const QModelIndex &index, int role) const
{
    if (!index.isValid()) return {};
    const int row = index.row();
    if (row < 0 || row >= static_cast<int>(m_visibleRows.size())) return {};
    const int rawIndex = m_visibleRows[static_cast<size_t>(row)];
    if (rawIndex < 0 || rawIndex >= static_cast<int>(m_rows.size())) return {};
    const Row &r = m_rows[static_cast<size_t>(rawIndex)];
    switch (role) {
    case KindRole: return static_cast<int>(r.kind);
    case TextRole: return r.text;
    case LevelRole: return static_cast<int>(r.level);
    default:       return {};
    }
}

QHash<int, QByteArray> PierLogStreamModel::roleNames() const
{
    return {
        { KindRole, "kind" },
        { TextRole, "text" },
        { LevelRole, "level" }
    };
}

void PierLogStreamModel::setStatus(Status s)
{
    if (m_status == s) return;
    m_status = s;
    emit statusChanged();
}

void PierLogStreamModel::setCommand(const QString &cmd)
{
    if (m_command == cmd) return;
    m_command = cmd;
    emit commandChanged();
}

void PierLogStreamModel::setFilterText(const QString &text)
{
    if (m_filterText == text) return;
    m_filterText = text;
    refreshFilterRegex();
    rebuildVisibleRows();
    emit filterStateChanged();
}

void PierLogStreamModel::setRegexMode(bool enabled)
{
    if (m_regexMode == enabled) return;
    m_regexMode = enabled;
    refreshFilterRegex();
    rebuildVisibleRows();
    emit filterStateChanged();
}

void PierLogStreamModel::setDebugEnabled(bool enabled)
{
    if (m_debugEnabled == enabled) return;
    m_debugEnabled = enabled;
    rebuildVisibleRows();
    emit levelFiltersChanged();
}

void PierLogStreamModel::setInfoEnabled(bool enabled)
{
    if (m_infoEnabled == enabled) return;
    m_infoEnabled = enabled;
    rebuildVisibleRows();
    emit levelFiltersChanged();
}

void PierLogStreamModel::setWarnEnabled(bool enabled)
{
    if (m_warnEnabled == enabled) return;
    m_warnEnabled = enabled;
    rebuildVisibleRows();
    emit levelFiltersChanged();
}

void PierLogStreamModel::setErrorEnabled(bool enabled)
{
    if (m_errorEnabled == enabled) return;
    m_errorEnabled = enabled;
    rebuildVisibleRows();
    emit levelFiltersChanged();
}

void PierLogStreamModel::setFatalEnabled(bool enabled)
{
    if (m_fatalEnabled == enabled) return;
    m_fatalEnabled = enabled;
    rebuildVisibleRows();
    emit levelFiltersChanged();
}

bool PierLogStreamModel::connectTo(const QString &host, int port, const QString &user,
                                    int authKind, const QString &secret, const QString &extra,
                                    const QString &command)
{
    if (m_handle || m_status == Connecting) {
        qWarning() << "PierLogStreamModel::connectTo called on already-connected session";
        return false;
    }
    if (host.isEmpty() || user.isEmpty() || command.isEmpty()
        || port <= 0 || port > 65535) {
        return false;
    }

    const quint64 requestId = ++m_nextRequestId;
    m_cancelFlag = std::make_shared<std::atomic<bool>>(false);
    m_errorMessage.clear();
    m_target = QStringLiteral("%1@%2:%3").arg(user, host).arg(port);
    setCommand(command);
    setStatus(Connecting);

    std::string hostStd = host.toStdString();
    std::string userStd = user.toStdString();
    std::string secretStd = secret.toStdString();
    std::string extraStd = extra.toStdString();
    std::string cmdStd = command.toStdString();
    const uint16_t portU16 = static_cast<uint16_t>(port);
    const int kind = authKind;

    QPointer<PierLogStreamModel> selfWeak(this);
    auto cancelFlag = m_cancelFlag;

    auto worker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId,
        hostStd = std::move(hostStd),
        userStd = std::move(userStd),
        secretStd = std::move(secretStd),
        extraStd = std::move(extraStd),
        cmdStd = std::move(cmdStd),
        portU16, kind
    ]() mutable {
        const char *secretPtr = secretStd.empty() ? nullptr : secretStd.c_str();
        const char *extraPtr  = extraStd.empty()  ? nullptr : extraStd.c_str();

        ::PierLogStream *h = pier_log_open(
            hostStd.c_str(),
            portU16,
            userStd.c_str(),
            kind,
            secretPtr,
            extraPtr,
            cmdStd.c_str());

        QString err;
        if (!h) {
            err = QStringLiteral("log open failed (see log)");
        }

        const bool cancelled = cancelFlag && cancelFlag->load();
        if (!selfWeak || cancelled) {
            if (h) pier_log_free(h);
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

void PierLogStreamModel::onConnectResult(quint64 requestId, void *handle, const QString &error)
{
    if (requestId != m_nextRequestId) {
        if (handle) pier_log_free(static_cast<::PierLogStream *>(handle));
        return;
    }
    if (!handle) {
        m_errorMessage = error.isEmpty() ? QStringLiteral("log open failed") : error;
        setStatus(Failed);
        return;
    }
    m_handle = static_cast<::PierLogStream *>(handle);
    setStatus(Connected);
    m_pollTimer.start();
}

void PierLogStreamModel::onPollTick()
{
    if (!m_handle) {
        m_pollTimer.stop();
        return;
    }

    // Drain JSON, parse, append. Drain returns NULL when no
    // events are pending — that's the normal case between
    // log bursts, not an error.
    char *json = pier_log_drain(m_handle);
    if (json) {
        const QString jsonStr = QString::fromUtf8(json);
        pier_log_free_string(json);
        ingestEventsJson(jsonStr);
    }

    // If the backend no longer has live data and we've
    // drained everything, transition to Finished and stop
    // polling. `is_alive` goes false on exit or error; the
    // final Exit event has already been ingested above.
    if (pier_log_is_alive(m_handle) == 0) {
        // Pull once more in case an event was enqueued
        // between the drain and the is_alive probe.
        char *tail = pier_log_drain(m_handle);
        if (tail) {
            const QString jsonStr = QString::fromUtf8(tail);
            pier_log_free_string(tail);
            ingestEventsJson(jsonStr);
        }
        m_exitCode = pier_log_exit_code(m_handle);
        m_pollTimer.stop();
        setStatus(Finished);
    }
}

void PierLogStreamModel::ingestEventsJson(const QString &json)
{
    QJsonParseError parseErr {};
    const QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8(), &parseErr);
    if (parseErr.error != QJsonParseError::NoError || !doc.isArray()) {
        qWarning() << "PierLogStreamModel: malformed drain JSON:" << parseErr.errorString();
        return;
    }
    const QJsonArray arr = doc.array();
    if (arr.isEmpty()) return;

    std::vector<Row> appended;
    appended.reserve(static_cast<size_t>(arr.size()));

    for (const QJsonValue &v : arr) {
        if (!v.isObject()) continue;
        const QJsonObject obj = v.toObject();
        const QString kindStr = obj.value(QStringLiteral("kind")).toString();
        Row row;
        if (kindStr == QStringLiteral("stdout")) {
            row.kind = Stdout;
            row.text = obj.value(QStringLiteral("text")).toString();
        } else if (kindStr == QStringLiteral("stderr")) {
            row.kind = Stderr;
            row.text = obj.value(QStringLiteral("text")).toString();
        } else if (kindStr == QStringLiteral("exit")) {
            row.kind = Exit;
            const int code = obj.value(QStringLiteral("exit_code")).toInt();
            m_exitCode = code;
            row.text = QStringLiteral("--- exited with code %1 ---").arg(code);
        } else if (kindStr == QStringLiteral("error")) {
            row.kind = ErrorKind;
            row.text = QStringLiteral("--- error: %1 ---")
                           .arg(obj.value(QStringLiteral("error")).toString());
        } else {
            // Unknown kind — skip rather than inventing a row.
            continue;
        }
        row.level = detectLevel(row.kind, row.text);
        appended.push_back(std::move(row));
    }

    if (appended.empty()) return;

    const int beforeVisible = static_cast<int>(m_visibleRows.size());
    std::vector<int> appendedVisible;
    appendedVisible.reserve(appended.size());
    for (auto &row : appended) {
        m_rows.push_back(std::move(row));
        const int rawIndex = static_cast<int>(m_rows.size()) - 1;
        if (rowMatchesFilters(m_rows.back())) {
            appendedVisible.push_back(rawIndex);
        }
    }
    if (!appendedVisible.empty()) {
        beginInsertRows(QModelIndex(),
                        beforeVisible,
                        beforeVisible + static_cast<int>(appendedVisible.size()) - 1);
        for (int rawIndex : appendedVisible) {
            m_visibleRows.push_back(rawIndex);
        }
        endInsertRows();
        emit lineCountChanged();
    }
    emit totalLineCountChanged();

    trimIfOverflow();
}

void PierLogStreamModel::trimIfOverflow()
{
    if (static_cast<int>(m_rows.size()) <= MAX_ROWS) return;
    const int overflow = static_cast<int>(m_rows.size()) - MAX_ROWS;
    beginResetModel();
    for (int i = 0; i < overflow; ++i) {
        m_rows.pop_front();
    }
    m_visibleRows.clear();
    m_visibleRows.reserve(m_rows.size());
    for (int i = 0; i < static_cast<int>(m_rows.size()); ++i) {
        if (rowMatchesFilters(m_rows[static_cast<size_t>(i)])) {
            m_visibleRows.push_back(i);
        }
    }
    endResetModel();
    emit lineCountChanged();
    emit totalLineCountChanged();
}

void PierLogStreamModel::clear()
{
    if (m_rows.empty() && m_visibleRows.empty()) return;
    beginResetModel();
    m_rows.clear();
    m_visibleRows.clear();
    endResetModel();
    emit lineCountChanged();
    emit totalLineCountChanged();
}

void PierLogStreamModel::stop()
{
    if (m_cancelFlag) {
        m_cancelFlag->store(true);
    }
    ++m_nextRequestId;
    m_pollTimer.stop();
    if (m_handle) {
        ::PierLogStream *h = m_handle;
        m_handle = nullptr;
        pier_log_stop(h);
        // Drain one last time so any tail events are
        // visible even if the user hit Stop before the
        // timer next fired.
        char *json = pier_log_drain(h);
        if (json) {
            const QString jsonStr = QString::fromUtf8(json);
            pier_log_free_string(json);
            ingestEventsJson(jsonStr);
        }
        pier_log_free(h);
    }
    if (m_status == Connecting || m_status == Connected) {
        setStatus(Finished);
    }
}

void PierLogStreamModel::rebuildVisibleRows()
{
    beginResetModel();
    m_visibleRows.clear();
    m_visibleRows.reserve(m_rows.size());
    for (int i = 0; i < static_cast<int>(m_rows.size()); ++i) {
        if (rowMatchesFilters(m_rows[static_cast<size_t>(i)])) {
            m_visibleRows.push_back(i);
        }
    }
    endResetModel();
    emit lineCountChanged();
}

void PierLogStreamModel::refreshFilterRegex()
{
    if (!m_regexMode || m_filterText.isEmpty()) {
        m_filterRegex = QRegularExpression();
        m_regexError.clear();
        return;
    }

    const QRegularExpression regex(
        m_filterText,
        QRegularExpression::CaseInsensitiveOption);
    if (regex.isValid()) {
        m_filterRegex = regex;
        m_regexError.clear();
    } else {
        m_filterRegex = QRegularExpression();
        m_regexError = regex.errorString();
    }
}

bool PierLogStreamModel::rowMatchesFilters(const Row &row) const
{
    switch (row.level) {
    case DebugLevel:
        if (!m_debugEnabled) return false;
        break;
    case InfoLevel:
        if (!m_infoEnabled) return false;
        break;
    case WarnLevel:
        if (!m_warnEnabled) return false;
        break;
    case ErrorLevel:
        if (!m_errorEnabled) return false;
        break;
    case FatalLevel:
        if (!m_fatalEnabled) return false;
        break;
    case UnknownLevel:
    default:
        break;
    }

    if (m_filterText.isEmpty()) {
        return true;
    }
    if (m_regexMode && m_filterRegex.isValid()) {
        return m_filterRegex.match(row.text).hasMatch();
    }
    return row.text.contains(m_filterText, Qt::CaseInsensitive);
}

PierLogStreamModel::Level PierLogStreamModel::detectLevel(Kind kind, const QString &text)
{
    const QString upper = text.toUpper();

    auto hasToken = [&upper](const QString &token) {
        return upper.contains(QStringLiteral("[%1]").arg(token))
            || upper.startsWith(token + QStringLiteral(" "))
            || upper.startsWith(token + QStringLiteral(":"));
    };

    if (hasToken(QStringLiteral("FATAL"))
        || hasToken(QStringLiteral("FTL"))
        || hasToken(QStringLiteral("PANIC"))) {
        return FatalLevel;
    }
    if (kind == ErrorKind) {
        return ErrorLevel;
    }
    if (hasToken(QStringLiteral("ERROR"))
        || hasToken(QStringLiteral("ERR"))
        || upper.contains(QStringLiteral(" ERROR "))
        || upper.contains(QStringLiteral(" FAILED"))
        || upper.contains(QStringLiteral(" FAILURE"))) {
        return ErrorLevel;
    }
    if (kind == Stderr) {
        return ErrorLevel;
    }
    if (hasToken(QStringLiteral("WARN"))
        || hasToken(QStringLiteral("WARNING"))
        || hasToken(QStringLiteral("WRN"))) {
        return WarnLevel;
    }
    if (hasToken(QStringLiteral("INFO"))
        || hasToken(QStringLiteral("INF"))) {
        return InfoLevel;
    }
    if (hasToken(QStringLiteral("DEBUG"))
        || hasToken(QStringLiteral("DBG"))
        || upper.contains(QStringLiteral(" TRACE"))) {
        return DebugLevel;
    }
    return UnknownLevel;
}
