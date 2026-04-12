#include "PierProfiler.h"

#include <QQmlEngine>

// ── Platform-specific memory query ───────────────────────
#if defined(Q_OS_MACOS) || defined(Q_OS_IOS)
#  include <mach/mach.h>
#elif defined(Q_OS_WIN)
#  include <windows.h>
#  include <psapi.h>
#elif defined(Q_OS_LINUX)
#  include <QFile>
#  include <QTextStream>
#endif

// Static members
qint64 PierProfiler::s_startupMs = 0;

// ── Construction ─────────────────────────────────────────

PierProfiler::PierProfiler(QObject *parent)
    : QObject(parent)
{
    // Read persisted preference.
    QSettings settings;
    m_enabled = settings.value(QStringLiteral("profiler/enabled"), false).toBool();

    // Sample timer fires every 500ms to push updated stats to QML.
    m_sampleTimer.setInterval(500);
    m_sampleTimer.setSingleShot(false);
    connect(&m_sampleTimer, &QTimer::timeout, this, &PierProfiler::onSampleTimer);

    if (m_enabled)
        m_sampleTimer.start();
}

PierProfiler *PierProfiler::create(QQmlEngine *, QJSEngine *)
{
    return new PierProfiler;
}

void PierProfiler::setStartupElapsed(qint64 ms)
{
    s_startupMs = ms;
}

// ── Property setters ─────────────────────────────────────

void PierProfiler::setEnabled(bool on)
{
    if (m_enabled == on)
        return;
    m_enabled = on;

    QSettings settings;
    settings.setValue(QStringLiteral("profiler/enabled"), on);

    if (on) {
        m_sampleTimer.start();
    } else {
        m_sampleTimer.stop();
    }

    emit enabledChanged();
}

// ── Window connection ────────────────────────────────────

void PierProfiler::connectToWindow(QQuickWindow *window)
{
    if (!window)
        return;

    // Use afterRendering to capture per-frame timestamps.
    // DirectConnection ensures we run on the render thread
    // (or gui thread if threaded rendering is off).
    connect(window, &QQuickWindow::afterRendering,
            this, &PierProfiler::onAfterRendering,
            Qt::DirectConnection);
}

// ── Frame timing ─────────────────────────────────────────

void PierProfiler::onAfterRendering()
{
    if (!m_enabled)
        return;

    if (!m_frameTimerStarted) {
        m_frameTimer.start();
        m_frameTimerStarted = true;
        return;
    }

    const qint64 elapsed = m_frameTimer.nsecsElapsed();
    m_frameTimer.start();

    m_frameTimes[m_frameIndex] = elapsed;
    m_frameIndex = (m_frameIndex + 1) % kRingSize;
    if (m_frameCount < kRingSize)
        ++m_frameCount;
}

// ── Periodic sampling ────────────────────────────────────

void PierProfiler::onSampleTimer()
{
    if (!m_enabled)
        return;

    // ── FPS ──
    if (m_frameCount > 1) {
        qint64 totalNs = 0;
        const int count = qMin(m_frameCount, kRingSize);
        for (int i = 0; i < count; ++i)
            totalNs += m_frameTimes[i];
        if (totalNs > 0) {
            const double avgFrameMs = static_cast<double>(totalNs) / count / 1e6;
            m_fps = qRound(1000.0 / avgFrameMs);
        }
    }

    // ── Memory ──
    const qint64 rss = queryRssBytes();
    m_memoryUsage = formatBytes(rss);

    emit statsUpdated();
}

// ── Platform queries ─────────────────────────────────────

qint64 PierProfiler::queryRssBytes()
{
#if defined(Q_OS_MACOS) || defined(Q_OS_IOS)
    mach_task_basic_info_data_t info{};
    mach_msg_type_number_t count = MACH_TASK_BASIC_INFO_COUNT;
    if (task_info(mach_task_self(), MACH_TASK_BASIC_INFO,
                  reinterpret_cast<task_info_t>(&info), &count) == KERN_SUCCESS) {
        return static_cast<qint64>(info.resident_size);
    }
    return 0;

#elif defined(Q_OS_WIN)
    PROCESS_MEMORY_COUNTERS pmc{};
    pmc.cb = sizeof(pmc);
    if (GetProcessMemoryInfo(GetCurrentProcess(), &pmc, sizeof(pmc)))
        return static_cast<qint64>(pmc.WorkingSetSize);
    return 0;

#elif defined(Q_OS_LINUX)
    QFile file(QStringLiteral("/proc/self/status"));
    if (file.open(QIODevice::ReadOnly | QIODevice::Text)) {
        QTextStream stream(&file);
        while (!stream.atEnd()) {
            const QString line = stream.readLine();
            if (line.startsWith(QLatin1String("VmRSS:"))) {
                // "VmRSS:    12345 kB"
                const QStringList parts = line.split(QLatin1Char(' '),
                                                      Qt::SkipEmptyParts);
                if (parts.size() >= 2)
                    return parts[1].toLongLong() * 1024; // kB → bytes
            }
        }
    }
    return 0;

#else
    return 0;
#endif
}

QString PierProfiler::formatBytes(qint64 bytes)
{
    if (bytes <= 0)
        return QStringLiteral("— MB");

    const double mb = static_cast<double>(bytes) / (1024.0 * 1024.0);
    if (mb < 1024.0)
        return QString::number(qRound(mb)) + QStringLiteral(" MB");

    const double gb = mb / 1024.0;
    return QString::number(gb, 'f', 1) + QStringLiteral(" GB");
}
