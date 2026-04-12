// ─────────────────────────────────────────────────────────
// PierI18n — runtime language switcher
// ─────────────────────────────────────────────────────────
//
// Exposes a `language` property to QML so the Settings dialog
// can present a locale picker. When the user switches, we:
//   1. Remove the old QTranslator
//   2. Load the new .qm file
//   3. Install it on QGuiApplication
//   4. Call engine->retranslate() so all qsTr() bindings update
//   5. Persist the choice to QSettings
//
// Available languages are discovered at compile time from the
// TS_FILES list in CMakeLists.txt. The "en" entry is always
// present (it means "no translator installed → source strings").

#pragma once

#include <QGuiApplication>
#include <QObject>
#include <QQmlApplicationEngine>
#include <QSettings>
#include <QString>
#include <QStringList>
#include <QTranslator>
#include <qqml.h>

class PierI18n : public QObject
{
    Q_OBJECT
    QML_NAMED_ELEMENT(PierI18n)
    QML_SINGLETON

    /// BCP-47-ish code: "en", "zh_CN", etc.
    Q_PROPERTY(QString language READ language NOTIFY languageChanged FINAL)

    /// Display names for the UI combo box, in the same order as `codes`.
    Q_PROPERTY(QStringList displayNames READ displayNames CONSTANT FINAL)

    /// Machine codes in the same order as `displayNames`.
    Q_PROPERTY(QStringList codes READ codes CONSTANT FINAL)

    /// Index into codes/displayNames for the current language.
    Q_PROPERTY(int currentIndex READ currentIndex NOTIFY languageChanged FINAL)

public:
    explicit PierI18n(QObject *parent = nullptr);

    // Singleton factory for QML (called by the engine).
    static PierI18n *create(QQmlEngine *engine, QJSEngine *);

    /// Call once from main() BEFORE engine.loadFromModule().
    /// Installs the saved translator so qsTr() works from the first QML evaluation.
    static void init(QGuiApplication *app, QQmlApplicationEngine *engine);

    /// Call once from main() AFTER engine.loadFromModule().
    /// Triggers retranslate() so any strings evaluated during load pick up translations.
    static void postInit();

    QString language() const { return m_language; }
    QStringList displayNames() const { return m_displayNames; }
    QStringList codes() const { return m_codes; }
    int currentIndex() const;

public slots:
    /// Switch to the given locale code (e.g. "zh_CN" or "en").
    void switchLanguage(const QString &code);

signals:
    void languageChanged();

private:
    bool loadTranslation(const QString &code);

    static QGuiApplication *s_app;
    static QQmlApplicationEngine *s_engine;

    QTranslator m_translator;
    bool m_translatorInstalled = false;
    QString m_language;

    QStringList m_codes;
    QStringList m_displayNames;
};
