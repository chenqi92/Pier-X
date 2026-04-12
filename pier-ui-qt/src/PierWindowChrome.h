#pragma once

#include <QObject>
#include <qqml.h>

class PierWindowChrome : public QObject
{
    Q_OBJECT
    QML_NAMED_ELEMENT(PierWindowChrome)
    QML_SINGLETON

public:
    enum TitleBarDoubleClickAction {
        NoAction = 0,
        MaximizeRestoreAction,
        MinimizeAction,
    };
    Q_ENUM(TitleBarDoubleClickAction)

    explicit PierWindowChrome(QObject *parent = nullptr);

    Q_INVOKABLE TitleBarDoubleClickAction titleBarDoubleClickAction() const;
    Q_INVOKABLE bool supportsSystemMenu() const;
    Q_INVOKABLE bool showSystemMenu(QObject *windowObject, qreal globalX, qreal globalY) const;
};
