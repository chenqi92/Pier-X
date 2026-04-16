mod app;
mod components;
mod data;
mod theme;
mod views;

use std::borrow::Cow;

use gpui::{
    px, size, App, Application, Bounds, KeyBinding, WindowBounds, WindowOptions,
};

use crate::app::{NewSshRequested, OpenLocalTerminalRequested, PierApp, ToggleTheme};

const INTER_VARIABLE: &[u8] = include_bytes!("../assets/fonts/InterVariable.ttf");
const JETBRAINS_MONO: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf");

fn main() {
    let app = Application::new();

    app.run(|cx: &mut App| {
        cx.text_system()
            .add_fonts(vec![
                Cow::Borrowed(INTER_VARIABLE),
                Cow::Borrowed(JETBRAINS_MONO),
            ])
            .expect("failed to load bundled fonts");

        theme::init(cx);

        cx.bind_keys([KeyBinding::new("cmd-shift-l", ToggleTheme, None)]);
        cx.on_action::<ToggleTheme>(|_, cx| theme::toggle(cx));
        cx.on_action::<NewSshRequested>(|_, _| {
            eprintln!("[pier] action: NewSshRequested (placeholder — wire to dialog in PR8)");
        });
        cx.on_action::<OpenLocalTerminalRequested>(|_, _| {
            eprintln!("[pier] action: OpenLocalTerminalRequested (placeholder — wire to terminal in PR6)");
        });

        let bounds = Bounds::centered(None, size(px(1100.0), px(760.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_window, cx| cx.new(|_| PierApp::new()),
        )
        .expect("failed to open GPUI window");
    });
}
