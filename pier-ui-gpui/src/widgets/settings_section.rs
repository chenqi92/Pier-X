//! Settings page section — flat row group under a small uppercase
//! title. The enclosing dialog already draws a rounded white card;
//! layering another bg_panel card on top created a "card-inside-a-
//! card" look that read as nested modals. The flat layout keeps
//! the grouped-list rhythm (SwiftUI `Form` / macOS System Settings)
//! without the extra chrome.
//!
//! Layout:
//!
//! ```text
//! TYPOGRAPHY           ← tertiary caption, uppercase when ASCII
//!   UI Font                          [Inter ▾]
//!   ─────────────────────────── border_subtle ──
//!   Terminal Font                    [Menlo ▾]
//! ```
//!
//! Use with [`crate::components::SettingRow`] for labeled rows. For
//! content that stands alone (a preview block, a 3-column theme
//! grid), use [`SettingsSection::untitled`] and pass the element
//! as a child — dividers are still auto-inserted between multiple
//! children, so mixing a preview and a row works as expected.

use gpui::{div, AnyElement, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window};

use crate::components::Separator;
use crate::theme::{
    spacing::SP_1,
    theme,
    typography::{SIZE_SMALL, WEIGHT_EMPHASIS},
};

#[derive(IntoElement, Default)]
pub struct SettingsSection {
    title: Option<SharedString>,
    children: Vec<AnyElement>,
}

impl SettingsSection {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: Some(title.into()),
            children: Vec::new(),
        }
    }

    /// Section without a header. Use for a top-of-page live preview
    /// or any block that stands on its own semantically. The card
    /// chrome still applies so it visually groups with titled
    /// sections below.
    pub fn untitled() -> Self {
        Self {
            title: None,
            children: Vec::new(),
        }
    }
}

impl ParentElement for SettingsSection {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for SettingsSection {
    fn render(self, _: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let t = theme(cx);

        let mut col = div().w_full().flex().flex_col().gap(SP_1);

        if let Some(title) = self.title {
            // Small, tertiary, uppercase-when-ASCII section title —
            // SwiftUI `Form` / macOS System Settings idiom. Sits
            // above a FLAT row group (no bg, no border) so the dialog
            // frame itself is the only container visible; the
            // earlier grouped-card surface produced a "card-inside-a
            // -card" look against the modal chrome.
            let label: SharedString = if title.as_ref().is_ascii() {
                SharedString::from(title.as_ref().to_uppercase())
            } else {
                title
            };
            col = col.child(
                div()
                    .text_size(SIZE_SMALL)
                    .font_weight(WEIGHT_EMPHASIS)
                    .text_color(t.color.text_tertiary)
                    .child(label),
            );
        }

        // Flat row group — rows stack directly on the dialog surface,
        // separated by subtle hairlines instead of a card fill. The
        // surrounding `page_shell` already provides horizontal
        // padding so the hairlines run full-bleed within the content
        // column.
        let mut body = div().w_full().flex().flex_col();

        for (ix, child) in self.children.into_iter().enumerate() {
            if ix > 0 {
                body = body.child(Separator::horizontal());
            }
            body = body.child(child);
        }

        col.child(body)
    }
}
