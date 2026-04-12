#include <QFontDatabase>
#include <QGuiApplication>
#include <QIcon>
#include <QLocale>
#include <QMetaObject>
#include <QQmlApplicationEngine>
#include <QQuickStyle>
#include <QStyleHints>
#include <QTranslator>
#include <QVariant>
#include <QWindow>

#include "PierNativeWindow.h"

namespace {

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
    QGuiApplication app(argc, argv);

    QGuiApplication::setApplicationName("Pier-X");
    QGuiApplication::setApplicationDisplayName("Pier-X");
    QGuiApplication::setApplicationVersion(QStringLiteral(PIER_X_VERSION));
    QGuiApplication::setOrganizationName("kkape");
    QGuiApplication::setOrganizationDomain("kkape.com");
    QGuiApplication::setWindowIcon(QIcon(QStringLiteral(":/qt/qml/Pier/resources/icons/pier.png")));

    // ── i18n ──────────────────────────────────────────
    // Load the compiled .qm translation that matches the system locale.
    // qt_add_translations embeds them under :/i18n/pier-x_<locale>.qm.
    QTranslator translator;
    const QLocale locale;
    if (translator.load(locale, QStringLiteral("pier-x"),
                        QStringLiteral("_"), QStringLiteral(":/i18n"))) {
        QGuiApplication::installTranslator(&translator);
    }

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

    QObject::connect(
        &engine,
        &QQmlApplicationEngine::objectCreationFailed,
        &app,
        []() { QCoreApplication::exit(-1); },
        Qt::QueuedConnection);

    engine.loadFromModule("Pier", "Main");

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
