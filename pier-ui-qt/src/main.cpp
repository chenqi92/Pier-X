#include <QElapsedTimer>
#include <QFontDatabase>
#include <QGuiApplication>
#include <QIcon>
#include <QLocale>
#include <QMetaObject>
#include <QQmlApplicationEngine>
#include <QQuickStyle>
#include <QStyleHints>
#include <QVariant>
#include <QWindow>

#include "PierI18n.h"
#include "PierNativeWindow.h"
#include "PierProfiler.h"
#include "PierUpdater.h"

namespace {

QString appIconPath()
{
#ifdef Q_OS_WIN
    // Windows title bars and taskbar buttons render the padded macOS asset too
    // small, so use a tighter crop there while keeping the current macOS icon.
    return QStringLiteral(":/qt/qml/Pier/resources/icons/pier-windows.png");
#else
    return QStringLiteral(":/qt/qml/Pier/resources/icons/pier.png");
#endif
}

// Push the OS color scheme into the QML Theme singleton.
// Called once at startup and again on every QStyleHints::colorSchemeChanged.
void syncSystemThemeToQml(QQmlApplicationEngine &engine)
{
    QObject *theme = engine.singletonInstance<QObject *>("Pier", "Theme");
    if (!theme) {
        return;
    }
    const bool dark =
        QGuiApplication::styleHints()->colorScheme() == Qt::ColorScheme::Dark;
    QMetaObject::invokeMethod(theme, "setSystemScheme", Q_ARG(QVariant, dark));
}

} // namespace

int main(int argc, char *argv[])
{
    // ── Startup timer ─────────────────────────────────
    // Capture wall-clock time from process start to QML ready.
    QElapsedTimer startupTimer;
    startupTimer.start();

    QGuiApplication app(argc, argv);

    QGuiApplication::setApplicationName("Pier-X");
    QGuiApplication::setApplicationDisplayName("Pier-X");
    QGuiApplication::setApplicationVersion(QStringLiteral(PIER_X_VERSION));
    QGuiApplication::setOrganizationName("kkape");
    QGuiApplication::setOrganizationDomain("kkape.com");
    QGuiApplication::setWindowIcon(QIcon(appIconPath()));

    // ── i18n ──────────────────────────────────────────
    // PierI18n is a QML_SINGLETON that manages the translator
    // lifecycle. We pass the app + engine pointers so the
    // singleton can install/remove QTranslator at runtime and
    // call engine->retranslate() when the user switches language.
    // The singleton itself is instantiated by the QML engine during
    // loadFromModule; init() just stores the pointers.

    // ── Bundled fonts ─────────────────────────────────
    // Guarantees Inter + JetBrains Mono are available even when
    // uninstalled on the user's system. Both are SIL OFL licensed.
    for (const auto &path : {
             QStringLiteral(":/qt/qml/Pier/resources/fonts/Inter-Regular.ttf"),
             QStringLiteral(":/qt/qml/Pier/resources/fonts/Inter-Medium.ttf"),
             QStringLiteral(":/qt/qml/Pier/resources/fonts/Inter-SemiBold.ttf"),
             QStringLiteral(":/qt/qml/Pier/resources/fonts/JetBrainsMono-Regular.ttf"),
             QStringLiteral(":/qt/qml/Pier/resources/fonts/JetBrainsMono-Medium.ttf"),
         }) {
        if (QFontDatabase::addApplicationFont(path) == -1)
            qWarning() << "Failed to load bundled font:" << path;
    }

    // Use the Basic style — we draw our own visuals via the Theme singleton,
    // so we don't want any platform style imposing colors on top of us.
    QQuickStyle::setStyle("Basic");

    QQmlApplicationEngine engine;

    // Give PierI18n access to app + engine before the QML tree
    // is instantiated — the singleton factory will use these to
    // install the initial translator matching the saved language.
    PierI18n::init(&app, &engine);

    QObject::connect(
        &engine,
        &QQmlApplicationEngine::objectCreationFailed,
        &app,
        []() { QCoreApplication::exit(-1); },
        Qt::QueuedConnection);

    engine.loadFromModule("Pier", "Main");

    // ── i18n post-init ───────────────────────────────
    // Retranslate the QML tree now that it's fully loaded.
    // This catches any qsTr() bindings that evaluated before
    // the translator was installed.
    PierI18n::postInit();

    // ── Startup profiling ─────────────────────────────
    PierProfiler::setStartupElapsed(startupTimer.elapsed());

    // ── Frameless chrome (macOS) ──────────────────────
    // Apply native window adjustments (transparent title bar +
    // traffic lights) to the root window created by the QML engine.
    // Must happen after loadFromModule since the window doesn't exist
    // until the QML tree has been instantiated.
    {
        const auto rootObjects = engine.rootObjects();
        for (auto *obj : rootObjects) {
            if (auto *w = qobject_cast<QWindow *>(obj)) {
                PierNativeWindow::applyFramelessChrome(w);
                break;
            }
        }
    }

    // ── Auto-update ───────────────────────────────────
    // Initialize platform-specific update framework (Sparkle on
    // macOS, WinSparkle on Windows). No-op on other platforms.
    PierUpdater::initialize();

    // Initial sync after the engine has loaded the singleton, then live-sync
    // on subsequent OS color scheme changes.
    syncSystemThemeToQml(engine);
    QObject::connect(
        QGuiApplication::styleHints(),
        &QStyleHints::colorSchemeChanged,
        &app,
        [&engine](Qt::ColorScheme) { syncSystemThemeToQml(engine); });

    return app.exec();
}
