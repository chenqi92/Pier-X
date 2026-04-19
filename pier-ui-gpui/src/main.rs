mod app;
mod assets;
mod components;
mod data;
mod diagnostics;
mod i18n;
mod platform;
mod theme;
mod ui_kit;
mod views;
mod widgets;

rust_i18n::i18n!("locales", fallback = "en");

use gpui::{
    point, px, size, App, AppContext, Application, Bounds, TitlebarOptions, WindowBounds,
    WindowOptions,
};
use gpui_component::Root;

use crate::app::{keybindings, PierApp, ToggleTheme};

const INTER_VARIABLE: &[u8] = include_bytes!("../assets/fonts/InterVariable.ttf");
const JETBRAINS_MONO: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf");

fn main() {
    if let Err(err) = diagnostics::init_logging() {
        eprintln!("[pier-x] failed to initialize diagnostics logging: {err}");
    }

    let app = Application::new().with_assets(assets::AppAssets);

    app.run(|cx: &mut App| {
        if let Some(path) = diagnostics::current_log_path() {
            log::info!("app bootstrap starting; diagnostics log={}", path.display());
        }

        i18n::init();

        cx.text_system()
            .add_fonts(vec![
                std::borrow::Cow::Borrowed(INTER_VARIABLE),
                std::borrow::Cow::Borrowed(JETBRAINS_MONO),
            ])
            .expect("failed to load bundled fonts");

        ui_kit::init(cx);
        theme::init(cx);
        ui_kit::sync_theme(cx);

        // Bind shortcuts from persisted settings — the Shortcuts tab
        // in the settings dialog calls `keybindings::apply_all` again
        // after saving a new assignment, so rebinds land immediately.
        let initial_settings = theme::current_settings(cx);
        keybindings::apply_all(cx, &initial_settings);

        // `on_action` registration is separate from the key binding
        // itself — the binding dispatches the action, these handlers
        // react to it. Shell-scoped actions (NewTab etc.) are wired
        // on `PierApp::render` via `cx.listener`.
        cx.on_action::<ToggleTheme>(|_, cx| {
            theme::toggle(cx);
            ui_kit::sync_theme(cx);
        });

        // Default size matches Pier (SwiftUI) for consistent first-run
        // impression; minimum size mirrors its `.defaultSize` floor so
        // the three-panel layout never collapses below a usable width.
        let bounds = Bounds::centered(None, size(px(1400.0), px(900.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(1200.0), px(700.0))),
                // macOS gets a unified title-bar: the system chrome is
                // transparent and the traffic-light cluster is parked
                // at y≈10 so it reads as sitting *inside* the 32px
                // toolbar rail. The toolbar pads its leading edge by
                // ~72px on macOS (see `app::toolbar::render`) so the
                // buttons don't collide with the traffic lights.
                //
                // On Windows / Linux the transparent titlebar still
                // removes the system title strip, and we draw our own
                // rail at the top of the shell — matching macOS.
                titlebar: Some(TitlebarOptions {
                    title: None,
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(12.0), px(10.0))),
                }),
                // Linux desktop environments use `app_id` to group windows and
                // map them to a desktop entry's icon. macOS / Windows ignore it:
                // macOS gets the dock icon from the `.app` bundle, while Windows
                // loads Win32 resource id 1 from the executable (compiled from
                // `pier-ui-gpui/app.rc` by `build.rs`).
                app_id: Some("com.pier-x.desktop".into()),
                ..Default::default()
            },
            |window, cx| {
                let view = cx.new(|app_cx| PierApp::new(window, app_cx));
                cx.new(|cx| Root::new(view, window, cx))
            },
        )
        .expect("failed to open GPUI window");
    });
}
