#pragma once

class QWindow;

// Platform-specific native window chrome adjustments.
// On macOS: hides the title bar text and background while keeping
// the native traffic light buttons (close / minimize / zoom).
// On Windows: no-op (default title bar is preserved).
namespace PierNativeWindow {

// Apply frameless chrome adjustments to the given window.
// Must be called after the window is created and visible.
void applyFramelessChrome(QWindow *window);

// Returns the height of the area reserved for native traffic lights
// (macOS) or zero on other platforms. The QML TopBar uses this to
// offset its left margin so its controls don't overlap the buttons.
int titleBarHeight();

} // namespace PierNativeWindow
