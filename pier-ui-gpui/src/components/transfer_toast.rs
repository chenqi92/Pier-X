//! Floating progress toast for the single in-flight SFTP transfer.
//!
//! Driven by [`crate::app::ssh_session::TransferState`]: the view
//! renders it as an elevated card pinned to the bottom-right, with a
//! thin accent progress bar, direction arrow, filename, bytes
//! transferred, and a live transfer rate. Done/failed states flip the
//! bar colour and swap the numerical readout for a short status line;
//! the session's auto-clear timer (see `schedule_sftp_mutation`) drops
//! the whole toast 1.5–2.5s after the final tick.
//!
//! Not wired as a `gpui_component::Notification` because the transfer
//! lifecycle differs from a toast: we want a stable position through
//! the entire transfer, and an animated bar that reflects byte-level
//! progress — both awkward to bolt onto a fire-and-forget
//! notification queue.

use gpui::{div, prelude::*, px, App, IntoElement, SharedString, Window};
use gpui_component::{Icon as UiIcon, IconName};

use crate::app::ssh_session::{TransferDirection, TransferPhase, TransferState};
use crate::theme::{
    heights::{GLYPH_SM, ICON_SM},
    radius::{RADIUS_MD, RADIUS_PILL},
    shadow,
    spacing::{SP_1, SP_1_5, SP_2, SP_3},
    theme,
    typography::{SIZE_CAPTION, SIZE_SMALL, WEIGHT_EMPHASIS, WEIGHT_MEDIUM},
};

/// Toast width. Narrow enough to sit unobtrusively above the status
/// bar; wide enough to give the filename room to breathe.
const TOAST_W: gpui::Pixels = px(280.0);
/// Progress bar height — thin is the whole point; we want a data-ink
/// accent line, not a block.
const BAR_H: gpui::Pixels = px(3.0);

#[derive(IntoElement)]
pub struct TransferToast {
    state: TransferState,
}

impl TransferToast {
    pub fn new(state: TransferState) -> Self {
        Self { state }
    }
}

impl RenderOnce for TransferToast {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme(cx);
        let TransferState {
            direction,
            name,
            transferred,
            total,
            phase,
            started_at,
            ..
        } = self.state;

        // Bar colour follows phase, not direction — the user cares
        // more about "is it working / did it land" than up-vs-down at
        // a glance.
        let bar_color = match phase {
            TransferPhase::Running => t.color.accent,
            TransferPhase::Done => t.color.status_success,
            TransferPhase::Failed => t.color.status_error,
        };

        let (fraction, indeterminate) = if total > 0 {
            let f = (transferred as f32 / total as f32).clamp(0.0, 1.0);
            // Done state fills the bar regardless of reported totals.
            (
                if matches!(phase, TransferPhase::Done) {
                    1.0
                } else {
                    f
                },
                false,
            )
        } else if matches!(phase, TransferPhase::Done | TransferPhase::Failed) {
            (1.0, false)
        } else {
            // Unknown total (download before first tick with size
            // metadata). Render a quarter-width pulse at the start —
            // still better than a stationary empty track.
            (0.25, true)
        };

        let arrow = match direction {
            TransferDirection::Upload => IconName::ArrowUp,
            TransferDirection::Download => IconName::ArrowDown,
        };

        let name_label: SharedString = name.into();
        let bytes_label: SharedString = format_bytes_label(transferred, total).into();
        let rate_label: SharedString = format_rate_label(transferred, started_at.elapsed()).into();
        let phase_label: Option<SharedString> = match phase {
            TransferPhase::Running => None,
            TransferPhase::Done => Some("done".into()),
            TransferPhase::Failed => Some("failed".into()),
        };

        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_1_5)
            .child(
                div()
                    .w(ICON_SM)
                    .h(ICON_SM)
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(bar_color)
                    .child(UiIcon::new(arrow).size(GLYPH_SM).text_color(bar_color)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .truncate()
                    .text_size(SIZE_CAPTION)
                    .font_weight(WEIGHT_MEDIUM)
                    .text_color(t.color.text_primary)
                    .child(name_label),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(SIZE_SMALL)
                    .font_weight(WEIGHT_EMPHASIS)
                    .text_color(t.color.text_secondary)
                    .child(match phase_label.clone() {
                        Some(label) => label,
                        None => format_percent(fraction).into(),
                    }),
            );

        // Progress bar: filled rect on a subtle track. When total is
        // unknown we still draw the 25% segment at the left so the
        // card doesn't look inert.
        let mut bar_fill = div()
            .h(BAR_H)
            .rounded(RADIUS_PILL)
            .bg(bar_color)
            .w(gpui::relative(fraction));
        if indeterminate {
            // Muted tint for the indeterminate pulse — the full
            // accent would overpromise precision we don't have.
            bar_fill = bar_fill.bg(t.color.accent_muted);
        }
        let bar = div()
            .w_full()
            .h(BAR_H)
            .rounded(RADIUS_PILL)
            .bg(t.color.accent_subtle)
            .child(bar_fill);

        let footer = div()
            .flex()
            .flex_row()
            .items_center()
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_size(SIZE_SMALL)
                    .text_color(t.color.text_tertiary)
                    .child(bytes_label),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(SIZE_SMALL)
                    .text_color(t.color.text_tertiary)
                    .child(rate_label),
            );

        div()
            .flex_none()
            .w(TOAST_W)
            .p(SP_3)
            .flex()
            .flex_col()
            .gap(SP_2)
            .bg(t.color.bg_elevated)
            .border_1()
            .border_color(t.color.border_subtle)
            .rounded(RADIUS_MD)
            .shadow(shadow::popover())
            .child(header)
            .child(bar)
            .child(footer)
            // 1px gutter between bytes/rate footer and the status bar
            // below so the toast doesn't optically merge with it.
            .child(div().h(SP_1).flex_none())
    }
}

/// Format "412 KB / 1.2 MB" (or just "412 KB" when total is unknown).
fn format_bytes_label(transferred: u64, total: u64) -> String {
    if total == 0 {
        format_bytes(transferred)
    } else {
        format!("{} / {}", format_bytes(transferred), format_bytes(total))
    }
}

fn format_bytes(n: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if n < KB {
        format!("{n} B")
    } else if n < MB {
        format!("{:.1} KB", n as f64 / KB as f64)
    } else if n < GB {
        format!("{:.1} MB", n as f64 / MB as f64)
    } else {
        format!("{:.2} GB", n as f64 / GB as f64)
    }
}

/// Render a transfer rate. Sub-second elapsed times are suppressed
/// (the number would be noise before we've actually moved anything
/// and the UI would flicker through huge denominators).
fn format_rate_label(transferred: u64, elapsed: std::time::Duration) -> String {
    let secs = elapsed.as_secs_f64();
    if secs < 0.25 || transferred == 0 {
        return "—".to_string();
    }
    let rate = (transferred as f64 / secs) as u64;
    format!("{}/s", format_bytes(rate))
}

fn format_percent(fraction: f32) -> String {
    format!("{:.0}%", fraction * 100.0)
}
