mod app;
mod assets;
mod components;
mod data;
mod diagnostics;
mod i18n;
mod theme;
mod ui_kit;
mod views;
mod widgets;

rust_i18n::i18n!("locales", fallback = "en");

use gpui::{px, size, App, AppContext, Application, Bounds, WindowBounds, WindowOptions};
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

        let bounds = Bounds::centered(None, size(px(1100.0), px(760.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
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
