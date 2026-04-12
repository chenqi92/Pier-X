#include "PierWindowChrome.h"

#include <QPoint>
#include <QtMath>
#include <QWindow>

#include "PierNativeWindow.h"

PierWindowChrome::PierWindowChrome(QObject *parent)
    : QObject(parent)
{
}

PierWindowChrome::TitleBarDoubleClickAction PierWindowChrome::titleBarDoubleClickAction() const
{
    switch (PierNativeWindow::titleBarDoubleClickAction()) {
    case PierNativeWindow::TitleBarDoubleClickAction::Minimize:
        return MinimizeAction;
    case PierNativeWindow::TitleBarDoubleClickAction::NoAction:
        return NoAction;
    case PierNativeWindow::TitleBarDoubleClickAction::MaximizeRestore:
    default:
        return MaximizeRestoreAction;
    }
}

bool PierWindowChrome::supportsSystemMenu() const
{
    return PierNativeWindow::supportsSystemMenu();
}

bool PierWindowChrome::showSystemMenu(QObject *windowObject, qreal globalX, qreal globalY) const
{
    auto *window = qobject_cast<QWindow *>(windowObject);
    if (!window)
        return false;

    return PierNativeWindow::showSystemMenu(
        window,
        QPoint(qRound(globalX), qRound(globalY)));
}
