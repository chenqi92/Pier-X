mod app;
mod assets;
mod components;
mod data;
mod theme;
mod ui_kit;
mod views;

use std::borrow::Cow;

use gpui::{
    px, size, App, AppContext, Application, Bounds, KeyBinding, TitlebarOptions, WindowBounds,
    WindowOptions,
};
use gpui_component::Root;

use crate::app::{
    CloseActiveTab, NewTab, PierApp, ToggleLeftPanel, ToggleRightPanel, ToggleTheme,
};

const INTER_VARIABLE: &[u8] = include_bytes!("../assets/fonts/InterVariable.ttf");
const JETBRAINS_MONO: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf");

fn main() {
    let app = Application::new().with_assets(assets::AppAssets);

    app.run(|cx: &mut App| {
        cx.text_system()
            .add_fonts(vec![
                Cow::Borrowed(INTER_VARIABLE),
                Cow::Borrowed(JETBRAINS_MONO),
            ])
            .expect("failed to load bundled fonts");

        ui_kit::init(cx);
        theme::init(cx);
        ui_kit::sync_theme(cx);

        // Global theme toggle (no key context — fires from anywhere).
        cx.bind_keys([KeyBinding::new("cmd-shift-l", ToggleTheme, None)]);
        cx.on_action::<ToggleTheme>(|_, cx| {
            theme::toggle(cx);
            ui_kit::sync_theme(cx);
        });

        // Shell-scoped shortcuts. The matching `on_action` handlers live on
        // `PierApp::render` so they have entity-state access via cx.listener.
        cx.bind_keys([
            KeyBinding::new("cmd-\\", ToggleLeftPanel, Some("PierApp")),
            KeyBinding::new("cmd-shift-\\", ToggleRightPanel, Some("PierApp")),
            KeyBinding::new("cmd-t", NewTab, Some("PierApp")),
            KeyBinding::new("cmd-shift-w", CloseActiveTab, Some("PierApp")),
        ]);

        let bounds = Bounds::centered(None, size(px(1100.0), px(760.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Pier-X".into()),
                    ..Default::default()
                }),
                // Linux desktop environments use `app_id` to group windows and
                // map them to a desktop entry's icon. macOS / Windows ignore it
                // — there the dock/taskbar icon comes from the `.app` / `.exe`
                // bundle (see `pier-ui-gpui/assets/app-icons/` and run.sh).
                app_id: Some("com.pier-x.desktop".into()),
                ..Default::default()
            },
            |window, cx| {
                let view = cx.new(|_| PierApp::new());
                cx.new(|cx| Root::new(view, window, cx))
            },
        )
        .expect("failed to open GPUI window");
    });
}
