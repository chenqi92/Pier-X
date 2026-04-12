#import <AppKit/AppKit.h>
#include <QWindow>

#include "PierNativeWindow.h"

namespace PierNativeWindow {

void applyFramelessChrome(QWindow *window)
{
    if (!window)
        return;

    // QWindow::winId() forces the native window to be created (if it
    // wasn't already) and returns its platform handle.  On macOS this
    // is an NSView*; its window property gives us the NSWindow*.
    auto *nsView = reinterpret_cast<NSView *>(window->winId());
    NSWindow *nsWindow = [nsView window];
    if (!nsWindow)
        return;

    // Hide the title bar text and icon.
    nsWindow.titleVisibility = NSWindowTitleHidden;

    // Make the title bar background transparent so our QML content
    // fills the entire window — the traffic light buttons are still
    // drawn by the system on top.
    nsWindow.titlebarAppearsTransparent = YES;

    // Extend the content view into the title bar area.  Combined
    // with the transparent title bar this gives us a seamless
    // "frameless with native buttons" look (the Visual Studio Code /
    // Warp / Linear style).
    nsWindow.styleMask |= NSWindowStyleMaskFullSizeContentView;

    // The toolbar style "unified" pushes the traffic lights down a
    // few pixels so they sit in our TopBar band rather than floating
    // at the very top edge.  This matches the IntelliJ / Linear
    // appearance.
    nsWindow.toolbarStyle = NSWindowToolbarStyleUnified;
}

int titleBarHeight()
{
    // The macOS traffic lights sit in a ~38 px tall band.  This is
    // the same height as our TopBar, so the QML layout aligns
    // naturally.  The value is hardcoded because reading it from
    // NSWindow requires a live window reference, and the QML
    // property is evaluated before the window exists.
    return 38;
}

} // namespace PierNativeWindow
