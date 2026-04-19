#![allow(dead_code)]

//! Page-level title block. The *grammar* of every working view:
//!
//! ```text
//! ┌──────────────────────────────────────────────┐
//! │ eyebrow                            [trailing] │
//! │ Title                                          │
//! │ subtitle_mono                        [status]  │
//! └──────────────────────────────────────────────┘
//! ```
//!
//! Intent — per the "default tacit layer" principle — is that a user
//! glancing at the page should know **what they're looking at** (title)
//! and **what they're being asked to do next** (trailing action) without
//! having to read body text. Everything visually quieter than the title
//! (eyebrow / subtitle / status) is context, not instruction.
//!
//! Two sizes:
//! - `HeaderSize::Page` — H3 title (`text::h3`), `PAGEHEADER_H` rail.
//!   The default for right-panel work modes (SSH / Git / DB / SFTP).
//! - `HeaderSize::Section` — SectionLabel eyebrow-style title, shorter.
//!   Use for left-panel group headers ("Files" / "Servers") that sit
//!   inside a containing pane. Same component, same builder — only the
//!   visual weight shrinks.
//!
//! Unfilled slots simply do not render. A minimal header is just a
//! title.

use gpui::{div, prelude::*, AnyElement, IntoElement, ParentElement, SharedString, Window};

use crate::components::{text, SectionLabel, StatusPill};
use crate::theme::{
    heights::PAGEHEADER_H,
    spacing::{SP_1, SP_2},
    theme,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HeaderSize {
    /// Full-weight page header — H3 title, `PAGEHEADER_H` rail. Default.
    Page,
    /// Compact group header — SectionLabel title, for panes inside a
    /// larger surface (left panel group).
    Section,
}

#[derive(IntoElement)]
pub struct PageHeader {
    size: HeaderSize,
    eyebrow: Option<SharedString>,
    title: SharedString,
    subtitle_mono: Option<SharedString>,
    status: Option<StatusPill>,
    trailing: Vec<AnyElement>,
}

impl PageHeader {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            size: HeaderSize::Page,
            eyebrow: None,
            title: title.into(),
            subtitle_mono: None,
            status: None,
            trailing: Vec::new(),
        }
    }

    pub fn size(mut self, size: HeaderSize) -> Self {
        self.size = size;
        self
    }

    pub fn eyebrow(mut self, eyebrow: impl Into<SharedString>) -> Self {
        self.eyebrow = Some(eyebrow.into());
        self
    }

    pub fn subtitle_mono(mut self, subtitle: impl Into<SharedString>) -> Self {
        self.subtitle_mono = Some(subtitle.into());
        self
    }

    pub fn status(mut self, pill: StatusPill) -> Self {
        self.status = Some(pill);
        self
    }

    pub fn trailing(mut self, child: impl IntoElement) -> Self {
        self.trailing.push(child.into_any_element());
        self
    }

    pub fn trailing_many(mut self, children: impl IntoIterator<Item = AnyElement>) -> Self {
        self.trailing.extend(children);
        self
    }
}

impl RenderOnce for PageHeader {
    fn render(self, _: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let t = theme(cx);

        // Title element — H2 for Page, UiLabel-ish (via SectionLabel
        // component) for Section. Always truncate: a header takes one
        // line by contract; overflow is always a mistake.
        let title_el: AnyElement = match self.size {
            HeaderSize::Page => text::h3(self.title.clone()).truncate().into_any_element(),
            HeaderSize::Section => SectionLabel::new(self.title.clone()).into_any_element(),
        };

        // Left column — eyebrow (optional) + title + subtitle (optional).
        let mut left = div().flex().flex_col().gap(SP_1).min_w_0().flex_1();
        if let Some(eyebrow) = self.eyebrow.as_ref() {
            // Eyebrow only makes sense for the full-weight header; for
            // `Section` the title itself already *is* the eyebrow.
            if matches!(self.size, HeaderSize::Page) {
                left = left.child(SectionLabel::new(eyebrow.clone()));
            }
        }
        left = left.child(title_el);
        if let Some(subtitle) = self.subtitle_mono {
            // Mono subtitles are usually a path or `user@host:port` —
            // long values are the norm. Truncate so the header never
            // wraps to two lines.
            left = left.child(text::mono(subtitle).secondary().truncate());
        }

        // Right column — trailing actions + optional status pill.
        let mut right = div().flex().flex_row().items_center().gap(SP_2).flex_none();
        if let Some(pill) = self.status {
            right = right.child(pill);
        }
        for action in self.trailing {
            right = right.child(action);
        }

        let height = match self.size {
            HeaderSize::Page => PAGEHEADER_H,
            HeaderSize::Section => crate::theme::heights::ROW_MD_H,
        };

        div()
            .w_full()
            .h(height)
            .px(SP_2)
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_2)
            .bg(t.color.bg_surface)
            .border_b_1()
            .border_color(t.color.border_subtle)
            .child(left)
            .child(right)
    }
}
