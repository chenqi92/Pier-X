//! Tiny modal dialog for the "New Group" action in the Servers
//! sidebar. Just a name input + Save/Cancel — groups have no
//! other attributes at this stage, they exist purely to act as
//! sidebar buckets that a connection's `tags[0]` can match
//! against.

use gpui::{div, prelude::*, px, App, Entity, IntoElement, SharedString, WeakEntity, Window};
use gpui_component::{
    input::{Input, InputState},
    WindowExt as _,
};
use rust_i18n::t;

use crate::app::PierApp;
use crate::theme::{
    spacing::{SP_1, SP_2},
    theme,
    typography::{SIZE_CAPTION, WEIGHT_MEDIUM},
};

/// Open the add-group dialog as a modal sheet.
pub fn open(window: &mut Window, cx: &mut App, app: WeakEntity<PierApp>) {
    // Input entity created outside the dialog closure so it
    // persists across re-renders (mirrors edit_connection::open).
    let name = cx.new(|c| {
        InputState::new(window, c).placeholder(t!("App.AddGroup.placeholder"))
    });
    let title: SharedString = t!("App.AddGroup.title").into();

    window.open_dialog(cx, move |dialog, _w, app_cx| {
        let body = build_body(app_cx, &name);
        let on_ok_name = name.clone();
        let weak = app.clone();
        dialog
            .title(title.clone())
            .w(px(360.0))
            .confirm()
            .button_props(
                gpui_component::dialog::DialogButtonProps::default()
                    .ok_text(t!("App.Common.save"))
                    .cancel_text(t!("App.Common.cancel")),
            )
            .on_ok(move |_, _w, app_cx| {
                let value = on_ok_name.read(app_cx).value().to_string();
                let trimmed = value.trim().to_string();
                if trimmed.is_empty() {
                    return true;
                }
                let _ = weak.update(app_cx, |pa, cx| {
                    pa.add_connection_group(trimmed, cx);
                });
                true
            })
            .child(body)
    });
}

fn build_body(cx: &App, name: &Entity<InputState>) -> impl IntoElement {
    let t = theme(cx).clone();
    div()
        .flex()
        .flex_col()
        .gap(SP_1)
        .pt(SP_2)
        .child(
            div()
                .text_size(SIZE_CAPTION)
                .font_weight(WEIGHT_MEDIUM)
                .text_color(t.color.text_secondary)
                .child(SharedString::from(
                    t!("App.AddGroup.field_label").to_string(),
                )),
        )
        .child(Input::new(name))
}
