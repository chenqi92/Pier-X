#include "PierNativeWindow.h"

#define NOMINMAX
#include <windows.h>

#include <QPoint>
#include <QWindow>

// Windows stub — keep the default system title bar.
// A custom frameless implementation (DWM + Snap Layouts) can be
// added here later if needed without touching the header or
// any other platform's file.

namespace PierNativeWindow {

void applyFramelessChrome(QWindow * /* window */)
{
    // no-op on Windows
}

int titleBarHeight()
{
    return 0;
}

TitleBarDoubleClickAction titleBarDoubleClickAction()
{
    return TitleBarDoubleClickAction::MaximizeRestore;
}

bool supportsSystemMenu()
{
    return true;
}

bool showSystemMenu(QWindow *window, const QPoint &globalPosition)
{
    if (!window)
        return false;

    const auto hwnd = reinterpret_cast<HWND>(window->winId());
    if (!hwnd)
        return false;

    HMENU menu = GetSystemMenu(hwnd, FALSE);
    if (!menu)
        return false;

    const bool maximized = IsZoomed(hwnd);
    const bool minimized = IsIconic(hwnd);

    EnableMenuItem(menu, SC_RESTORE, MF_BYCOMMAND | ((maximized || minimized) ? MF_ENABLED : MF_GRAYED));
    EnableMenuItem(menu, SC_MOVE, MF_BYCOMMAND | ((!maximized && !minimized) ? MF_ENABLED : MF_GRAYED));
    EnableMenuItem(menu, SC_SIZE, MF_BYCOMMAND | ((!maximized && !minimized) ? MF_ENABLED : MF_GRAYED));
    EnableMenuItem(menu, SC_MINIMIZE, MF_BYCOMMAND | (minimized ? MF_GRAYED : MF_ENABLED));
    EnableMenuItem(menu, SC_MAXIMIZE, MF_BYCOMMAND | (maximized ? MF_GRAYED : MF_ENABLED));

    const auto command = TrackPopupMenu(
        menu,
        TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN | TPM_RIGHTBUTTON,
        globalPosition.x(),
        globalPosition.y(),
        0,
        hwnd,
        nullptr);
    if (command == 0)
        return false;

    PostMessageW(hwnd, WM_SYSCOMMAND, static_cast<WPARAM>(command), 0);
    return true;
}

} // namespace PierNativeWindow
