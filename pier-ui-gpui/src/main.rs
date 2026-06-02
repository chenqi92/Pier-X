// Pier-X GPUI spike — entry point.
//
// Opens a window rendering the shell skeleton (see shell.rs) painted from the
// ported design tokens (theme.rs). This is the "does GPUI look ugly?" eyeball
// build, not a functional app. See docs/GPUI-MIGRATION-PLAN.md (M0).

mod shell;
mod theme;

use gpui::{
    point, px, size, App, AppContext, Bounds, Styled, TitlebarOptions, WindowBounds, WindowOptions,
};
use gpui_component::{ActiveTheme, Root};

use shell::Shell;
use theme::Theme;

fn main() {
    gpui_platform::application().run(move |cx: &mut App| {
        gpui_component::init(cx);
        cx.set_global(Theme::dark());

        cx.spawn(async move |cx| {
            let bounds = Bounds {
                origin: point(px(160.0), px(100.0)),
                size: size(px(1240.0), px(800.0)),
            };
            let opts = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Pier-X".into()),
                    ..Default::default()
                }),
                ..Default::default()
            };
            cx.open_window(opts, |window, cx| {
                let view = cx.new(|_| Shell::new());
                cx.new(|cx| Root::new(view, window, cx).bg(cx.theme().background))
            })
            .expect("failed to open window");
        })
        .detach();
    });
}
