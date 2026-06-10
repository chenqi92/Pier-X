// Pier-X GPUI spike — entry point.
//
// Opens a window rendering the shell skeleton (see shell.rs) painted from the
// ported design tokens (theme.rs). This is the "does GPUI look ugly?" eyeball
// build, not a functional app. See docs/GPUI-MIGRATION-PLAN.md (M0).

mod assets;
mod data;
mod dialogs;
mod git_panel;
mod i18n;
mod panels;
mod settings;
mod shell;
mod terminal;
mod theme;
mod ui;

use gpui::{
    point, px, size, App, AppContext, Bounds, KeyBinding, Styled, WindowBounds, WindowOptions,
};
use gpui_component::{ActiveTheme, Root, TitleBar};

use assets::Assets;
use shell::{
    CmdCloseTab, CmdNewTerminal, CmdPalette, CmdSettings, CmdToggleTheme, Shell,
};
use theme::Theme;

fn main() {
    gpui_platform::application().with_assets(Assets).run(move |cx: &mut App| {
        gpui_component::init(cx);
        if let Err(e) = assets::load_fonts(cx) {
            eprintln!("failed to load embedded fonts: {e}");
        }
        // Global keyboard shortcuts (handled on the shell root via on_action).
        cx.bind_keys([
            KeyBinding::new("ctrl-shift-p", CmdPalette, None),
            KeyBinding::new("ctrl-shift-t", CmdNewTerminal, None),
            KeyBinding::new("ctrl-shift-w", CmdCloseTab, None),
            KeyBinding::new("ctrl-shift-l", CmdToggleTheme, None),
            KeyBinding::new("ctrl-,", CmdSettings, None),
        ]);
        cx.set_global(Theme::dark());
        // Sync gpui-component's own theme (drives the TitleBar window-control
        // icon colours) to match; without this the min/max/close icons render in
        // the light-default foreground and vanish on the dark title bar.
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);

        cx.spawn(async move |cx| {
            let bounds = Bounds {
                origin: point(px(160.0), px(100.0)),
                size: size(px(1240.0), px(800.0)),
            };
            let opts = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                // Client-side decorations: gpui-component's TitleBar draws the
                // chrome (drag + min/max/close) so the shell owns the whole frame.
                titlebar: Some(TitleBar::title_bar_options()),
                ..Default::default()
            };
            cx.open_window(opts, |window, cx| {
                window.activate_window();
                window.set_window_title("Pier-X");
                let view = cx.new(|cx| Shell::new(cx));
                cx.new(|cx| Root::new(view, window, cx).bg(cx.theme().background))
            })
            .expect("failed to open window");
        })
        .detach();
    });
}
