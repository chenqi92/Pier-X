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
use crate::components::{FormField, FormSection};
use crate::theme::spacing::SP_2;

/// Open the add-group dialog as a modal sheet.
pub fn open(window: &mut Window, cx: &mut App, app: WeakEntity<PierApp>) {
    // Input entity created outside the dialog closure so it
    // persists across re-renders (mirrors edit_connection::open).
    let name = cx.new(|c| InputState::new(window, c).placeholder(t!("App.AddGroup.placeholder")));
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

fn build_body(_cx: &App, name: &Entity<InputState>) -> impl IntoElement {
    div().w_full().flex().flex_col().gap(SP_2).pt(SP_2).child(
        FormSection::untitled()
            .child(FormField::new(t!("App.AddGroup.field_label")).child(Input::new(name))),
    )
}
