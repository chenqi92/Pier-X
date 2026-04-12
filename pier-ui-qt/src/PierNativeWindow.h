#pragma once

class QWindow;
class QPoint;

// Platform-specific native window chrome adjustments.
// On macOS: hides the title bar text and background while keeping
// the native traffic light buttons (close / minimize / zoom).
// On Windows: no-op (default title bar is preserved).
namespace PierNativeWindow {

enum class TitleBarDoubleClickAction {
    NoAction = 0,
    MaximizeRestore,
    Minimize,
};

// Apply frameless chrome adjustments to the given window.
// Must be called after the window is created and visible.
void applyFramelessChrome(QWindow *window);

// Returns the height of the area reserved for native traffic lights
// (macOS) or zero on other platforms. The QML TopBar uses this to
// offset its left margin so its controls don't overlap the buttons.
int titleBarHeight();

// Returns the platform-preferred title-bar double-click behavior.
TitleBarDoubleClickAction titleBarDoubleClickAction();

// Whether the platform exposes a native title-bar system menu that we
// can surface from the custom TopBar area.
bool supportsSystemMenu();

// Shows the platform-native title-bar system menu at the given global
// screen position. Returns true if the menu was shown.
bool showSystemMenu(QWindow *window, const QPoint &globalPosition);

} // namespace PierNativeWindow
