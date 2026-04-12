// ─────────────────────────────────────────────────────────
// PierProfiler — lightweight runtime performance monitor
// ─────────────────────────────────────────────────────────
//
// Exposes live FPS, RSS memory usage, and startup elapsed time
// to QML via a singleton.  The StatusBar shows these metrics
// when the user enables the performance overlay in Settings.
//
// Metrics are sampled on a 500ms QTimer.  FPS is derived from
// QQuickWindow::afterRendering signal timestamps.  Memory is
// read from platform APIs (mach_task_info / GetProcessMemoryInfo
// / /proc/self/status).

#pragma once

#include <QElapsedTimer>
#include <QObject>
#include <QQuickWindow>
#include <QSettings>
#include <QString>
#include <QTimer>
#include <qqml.h>

class PierProfiler : public QObject
{
    Q_OBJECT
    QML_NAMED_ELEMENT(PierProfiler)
    QML_SINGLETON

    /// Smoothed frames-per-second (rolling average over ~60 frames).
    Q_PROPERTY(int fps READ fps NOTIFY statsUpdated FINAL)

    /// Human-readable RSS memory string, e.g. "142 MB".
    Q_PROPERTY(QString memoryUsage READ memoryUsage NOTIFY statsUpdated FINAL)

    /// Milliseconds from main() to QML Component.onCompleted.
    Q_PROPERTY(int startupMs READ startupMs CONSTANT FINAL)

    /// Whether the profiler collects and displays metrics.
    Q_PROPERTY(bool enabled READ enabled WRITE setEnabled NOTIFY enabledChanged FINAL)

public:
    explicit PierProfiler(QObject *parent = nullptr);

    // QML singleton factory.
    static PierProfiler *create(QQmlEngine *engine, QJSEngine *);

    // Called once from main() to record the startup timer result.
    static void setStartupElapsed(qint64 ms);

    int fps() const { return m_fps; }
    QString memoryUsage() const { return m_memoryUsage; }
    int startupMs() const { return static_cast<int>(s_startupMs); }
    bool enabled() const { return m_enabled; }
    void setEnabled(bool on);

    /// Connect to a QQuickWindow to receive afterRendering signals.
    /// Called from Main.qml Component.onCompleted.
    Q_INVOKABLE void connectToWindow(QQuickWindow *window);

signals:
    void statsUpdated();
    void enabledChanged();

private slots:
    void onAfterRendering();
    void onSampleTimer();

private:
    static qint64 queryRssBytes();
    static QString formatBytes(qint64 bytes);

    static qint64 s_startupMs;

    bool m_enabled = false;
    int m_fps = 0;
    QString m_memoryUsage;

    // Frame timing ring buffer
    QElapsedTimer m_frameTimer;
    bool m_frameTimerStarted = false;
    static constexpr int kRingSize = 64;
    qint64 m_frameTimes[kRingSize] = {};
    int m_frameIndex = 0;
    int m_frameCount = 0;

    QTimer m_sampleTimer;
};
