#import <AppKit/AppKit.h>
#import <Foundation/Foundation.h>
#include <QPoint>
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
    auto *nsView = (__bridge NSView *)(reinterpret_cast<void *>(window->winId()));
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

    // Let the transparent title-bar/background band participate in
    // standard window dragging. Without this, only explicit QML
    // startSystemMove() areas drag reliably, and the top edge can
    // feel "dead" unless the user happens to hit the resize hotspot.
    nsWindow.movableByWindowBackground = YES;

    // Keep the traffic lights compact so the QML top bar content sits
    // on the same visual row instead of reading as a second line below.
    nsWindow.toolbarStyle = NSWindowToolbarStyleUnifiedCompact;

    // Preserve native fullscreen transitions with our custom title bar.
    nsWindow.collectionBehavior |= NSWindowCollectionBehaviorFullScreenPrimary;
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

TitleBarDoubleClickAction titleBarDoubleClickAction()
{
    NSUserDefaults *defaults = [NSUserDefaults standardUserDefaults];

    id configuredAction = [defaults objectForKey:@"AppleActionOnDoubleClick"];
    if ([configuredAction isKindOfClass:[NSString class]]) {
        NSString *action = [(NSString *)configuredAction lowercaseString];
        if ([action isEqualToString:@"minimize"])
            return TitleBarDoubleClickAction::Minimize;
        if ([action isEqualToString:@"maximize"] ||
            [action isEqualToString:@"zoom"])
            return TitleBarDoubleClickAction::MaximizeRestore;
        if ([action isEqualToString:@"none"] ||
            [action isEqualToString:@"do nothing"])
            return TitleBarDoubleClickAction::NoAction;
    }

    // Older macOS releases stored the setting as a boolean.
    if ([defaults objectForKey:@"AppleMiniaturizeOnDoubleClick"]) {
        return [defaults boolForKey:@"AppleMiniaturizeOnDoubleClick"]
            ? TitleBarDoubleClickAction::Minimize
            : TitleBarDoubleClickAction::MaximizeRestore;
    }

    return TitleBarDoubleClickAction::MaximizeRestore;
}

bool supportsSystemMenu()
{
    return false;
}

bool showSystemMenu(QWindow * /* window */, const QPoint & /* globalPosition */)
{
    return false;
}

} // namespace PierNativeWindow
