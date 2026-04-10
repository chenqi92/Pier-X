#include <QGuiApplication>
#include <QIcon>
#include <QQmlApplicationEngine>
#include <QQuickStyle>
#include <QStyleHints>

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

    return app.exec();
}
