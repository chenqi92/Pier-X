#include <QGuiApplication>
#include <QIcon>
#include <QMetaObject>
#include <QQmlApplicationEngine>
#include <QQuickStyle>
#include <QStyleHints>
#include <QVariant>

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
