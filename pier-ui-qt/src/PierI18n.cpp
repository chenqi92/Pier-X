#include "PierI18n.h"

#include <QLocale>
#include <QQmlEngine>

// Static members — set once from main().
QGuiApplication *PierI18n::s_app = nullptr;
QQmlApplicationEngine *PierI18n::s_engine = nullptr;

// ── Singleton plumbing ───────────────────────────────────

PierI18n::PierI18n(QObject *parent)
    : QObject(parent)
{
    // Supported locales — add new entries here as you add .ts files.
    m_codes        = { QStringLiteral("en"),      QStringLiteral("zh_CN") };
    m_displayNames = { QStringLiteral("English"), QStringLiteral("简体中文") };

    // Read persisted choice (default = follow system).
    QSettings settings;
    const QString saved = settings.value(QStringLiteral("i18n/language"),
                                         QStringLiteral("system")).toString();

    if (saved == QStringLiteral("system") || saved.isEmpty()) {
        // Auto-detect from system locale.
        const QString sysName = QLocale::system().name(); // e.g. "zh_CN"
        if (m_codes.contains(sysName))
            m_language = sysName;
        else
            m_language = QStringLiteral("en");
    } else {
        m_language = saved;
    }
}

PierI18n *PierI18n::create(QQmlEngine * /*engine*/, QJSEngine *)
{
    auto *instance = new PierI18n;
    // The translator was already loaded by init() in main.cpp.
    // We only need to load our own translator if we want switchLanguage()
    // to work (it removes/reinstalls m_translator, not the static one).
    // Load into m_translator so future switches work correctly.
    if (instance->m_language != QStringLiteral("en")) {
        instance->loadTranslation(instance->m_language);
    }
    return instance;
}

void PierI18n::init(QGuiApplication *app, QQmlApplicationEngine *engine)
{
    s_app = app;
    s_engine = engine;

    // Load the saved translation BEFORE the QML tree is instantiated,
    // so the very first qsTr() evaluations during loadFromModule()
    // pick up the correct language.
    QSettings settings;
    const QString saved = settings.value(QStringLiteral("i18n/language"),
                                         QStringLiteral("system")).toString();
    QString code;
    if (saved == QStringLiteral("system") || saved.isEmpty()) {
        const QString sysName = QLocale::system().name();
        // Check if we have a translation for the system locale
        QTranslator probe;
        if (probe.load(QStringLiteral("pier-x_") + sysName, QStringLiteral(":/i18n")))
            code = sysName;
        else
            code = QStringLiteral("en");
    } else {
        code = saved;
    }

    if (code != QStringLiteral("en")) {
        // Install a static translator that lives for the app's lifetime.
        // The PierI18n singleton (created later by QML) will take over management.
        static QTranslator earlyTranslator;
        if (earlyTranslator.load(QStringLiteral("pier-x_") + code, QStringLiteral(":/i18n"))) {
            app->installTranslator(&earlyTranslator);
            qInfo() << "PierI18n: loaded translation for" << code;
        } else {
            qWarning() << "PierI18n: failed to load translation for" << code;
        }
    }
}

void PierI18n::postInit()
{
    // After the QML tree is fully loaded, retranslate once to catch
    // any bindings that were set up with the wrong language.
    if (s_engine) {
        s_engine->retranslate();
        // retranslated
    }
}

// ── Public API ───────────────────────────────────────────

int PierI18n::currentIndex() const
{
    const int idx = m_codes.indexOf(m_language);
    return idx >= 0 ? idx : 0;
}

void PierI18n::switchLanguage(const QString &code)
{
    if (code == m_language)
        return;

    // Remove the old translator if installed.
    if (m_translatorInstalled && s_app) {
        s_app->removeTranslator(&m_translator);
        m_translatorInstalled = false;
    }

    if (code == QStringLiteral("en")) {
        // English = source strings, no translator needed.
        m_language = code;
    } else if (loadTranslation(code)) {
        m_language = code;
    } else {
        // Fallback to English if loading failed.
        m_language = QStringLiteral("en");
    }

    // Persist the choice.
    QSettings settings;
    settings.setValue(QStringLiteral("i18n/language"), m_language);
    settings.sync();

    emit languageChanged();

    if (s_engine)
        s_engine->retranslate();
}

// ── Internals ────────────────────────────────────────────

bool PierI18n::loadTranslation(const QString &code)
{
    if (!s_app)
        return false;

    // The .qm files are embedded via qt_add_translations with
    // RESOURCE_PREFIX /i18n, so they live at :/i18n/pier-x_<code>.qm.
    const bool ok = m_translator.load(
        QStringLiteral("pier-x_") + code,
        QStringLiteral(":/i18n"));

    if (!ok) {
        qWarning() << "PierI18n: failed to load translation for" << code
                    << "from :/i18n/pier-x_" + code;
        return false;
    }

    s_app->installTranslator(&m_translator);
    m_translatorInstalled = true;
    qWarning() << "PierI18n: loaded translation for" << code << "successfully,"
               << "isEmpty=" << m_translator.isEmpty();
    return true;
}
