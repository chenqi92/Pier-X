use gpui::{
    div, prelude::*, px, App, ClickEvent, Context, InteractiveElement, IntoElement, SharedString,
    StatefulInteractiveElement, WeakEntity, Window,
};
use pier_core::settings::{AppSettings, AppearanceMode, TerminalCursorStyle, TerminalThemePreset};

use gpui_component::{scroll::ScrollableElement, WindowExt as _};

use crate::theme::{
    available_terminal_font_families,
    radius::RADIUS_SM,
    spacing::{SP_1, SP_1_5, SP_2, SP_3, SP_4},
    terminal::{available_terminal_palettes, terminal_bg_color, terminal_hex_color},
    terminal_cursor_blink, terminal_cursor_style, terminal_font_for_family,
    terminal_font_ligatures, terminal_font_size, terminal_opacity, theme,
    typography::{SIZE_BODY, SIZE_CAPTION, SIZE_H2, SIZE_SMALL, WEIGHT_EMPHASIS, WEIGHT_MEDIUM},
    update_settings,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsSection {
    General,
    Terminal,
    Shortcuts,
}

impl SettingsSection {
    const ALL: [Self; 3] = [Self::General, Self::Terminal, Self::Shortcuts];

    fn title(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Terminal => "Terminal",
            Self::Shortcuts => "Shortcuts",
        }
    }

    fn caption(self) -> &'static str {
        match self {
            Self::General => "Appearance and shell defaults",
            Self::Terminal => "Terminal rendering and cursor behavior",
            Self::Shortcuts => "Current built-in key bindings",
        }
    }
}

pub struct SettingsDialog {
    selected_section: SettingsSection,
    entity: WeakEntity<SettingsDialog>,
}

impl SettingsDialog {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            selected_section: SettingsSection::General,
            entity: cx.entity().downgrade(),
        }
    }

    fn select_section(&mut self, section: SettingsSection, cx: &mut Context<Self>) {
        if self.selected_section != section {
            self.selected_section = section;
            cx.notify();
        }
    }
}

impl Render for SettingsDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();

        div()
            .w(px(760.0))
            .h(px(500.0))
            .flex()
            .flex_row()
            .bg(t.color.bg_surface)
            .child(self.render_sidebar(&t, cx))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .h_full()
                    .bg(t.color.bg_surface)
                    .child(self.render_content(cx)),
            )
    }
}

impl SettingsDialog {
    fn render_sidebar(&self, t: &crate::theme::Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let mut col = div()
            .w(px(208.0))
            .h_full()
            .px(SP_2)
            .py(SP_3)
            .flex()
            .flex_col()
            .gap(SP_1)
            .bg(t.color.bg_panel)
            .border_r_1()
            .border_color(t.color.border_subtle)
            .child(
                div()
                    .px(SP_2)
                    .pb(SP_2)
                    .border_b_1()
                    .border_color(t.color.border_subtle)
                    .child(
                        div()
                            .text_size(SIZE_H2)
                            .font_weight(WEIGHT_EMPHASIS)
                            .text_color(t.color.text_primary)
                            .child("Settings"),
                    )
                    .child(
                        div()
                            .pt(px(4.0))
                            .text_size(SIZE_SMALL)
                            .text_color(t.color.text_tertiary)
                            .child("Ported from the sibling Pier app where it already exists."),
                    ),
            );

        for section in SettingsSection::ALL {
            let is_active = section == self.selected_section;
            let on_click = cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.select_section(section, cx);
            });

            col = col.child(
                div()
                    .id(section.title())
                    .px(SP_2)
                    .py(SP_2)
                    .rounded(RADIUS_SM)
                    .cursor_pointer()
                    .bg(if is_active {
                        t.color.accent_subtle
                    } else {
                        t.color.bg_panel
                    })
                    .text_color(if is_active {
                        t.color.accent
                    } else {
                        t.color.text_secondary
                    })
                    .hover(|style| style.bg(t.color.bg_hover))
                    .on_click(on_click)
                    .child(
                        div()
                            .text_size(SIZE_BODY)
                            .font_weight(WEIGHT_MEDIUM)
                            .child(section.title()),
                    )
                    .child(
                        div()
                            .pt(px(3.0))
                            .text_size(SIZE_SMALL)
                            .text_color(if is_active {
                                t.color.text_secondary
                            } else {
                                t.color.text_tertiary
                            })
                            .child(section.caption()),
                    ),
            );
        }

        col
    }

    fn render_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        match self.selected_section {
            SettingsSection::General => self.render_general(cx).into_any_element(),
            SettingsSection::Terminal => self.render_terminal(cx).into_any_element(),
            SettingsSection::Shortcuts => self.render_shortcuts(cx).into_any_element(),
        }
    }

    fn render_general(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let appearance = t.settings.appearance_mode;
        let mono_font = t.settings.terminal_font_family.clone();
        let dialog = self.entity.clone();

        section_shell(
            &t,
            "General",
            "Shell-level preferences that already map cleanly to GPUI.",
        )
        .child(setting_group(
            &t,
            "Appearance",
            vec![
                choice_chip(
                    &t,
                    "Dark",
                    appearance == AppearanceMode::Dark,
                    Box::new({
                        let dialog = dialog.clone();
                        move |_, _, app| {
                            apply_dialog_settings(&dialog, app, |settings| {
                                settings.appearance_mode = AppearanceMode::Dark;
                            });
                        }
                    }),
                )
                .into_any_element(),
                choice_chip(
                    &t,
                    "Light",
                    appearance == AppearanceMode::Light,
                    Box::new({
                        let dialog = dialog.clone();
                        move |_, _, app| {
                            apply_dialog_settings(&dialog, app, |settings| {
                                settings.appearance_mode = AppearanceMode::Light;
                            });
                        }
                    }),
                )
                .into_any_element(),
            ],
        ))
        .child(setting_group(
            &t,
            "Terminal Font Family",
            available_terminal_font_families()
                .iter()
                .map(|family| {
                    let family_name = (*family).to_string();
                    choice_chip(
                        &t,
                        family,
                        mono_font == family_name,
                        Box::new({
                            let dialog = dialog.clone();
                            move |_, _, app| {
                                let family_name = family_name.clone();
                                apply_dialog_settings(&dialog, app, move |settings| {
                                    settings.terminal_font_family = family_name.clone();
                                });
                            }
                        }),
                    )
                    .into_any_element()
                })
                .collect(),
        ))
    }

    fn render_terminal(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let font_size = terminal_font_size(cx);
        let cursor_style = terminal_cursor_style(cx);
        let cursor_blink = terminal_cursor_blink(cx);
        let font_ligatures = terminal_font_ligatures(cx);
        let theme_preset = t.settings.terminal_theme_preset;
        let opacity_pct = t.settings.terminal_opacity_pct;
        let opacity = terminal_opacity(cx);
        let palette = crate::theme::terminal::terminal_palette(theme_preset);
        let preview_font = terminal_font_for_family(&t.font_mono, font_ligatures);
        let preview_size = px(font_size);
        let preview_bg = terminal_bg_color(palette.background_hex, opacity);
        let preview_fg = terminal_hex_color(palette.foreground_hex);
        let dialog = self.entity.clone();

        section_shell(
            &t,
            "Terminal",
            "Settings below are applied immediately to the active GPUI terminal surface.",
        )
        .child(setting_group(
            &t,
            "Theme",
            available_terminal_palettes()
                .iter()
                .map(|palette| {
                    let palette = *palette;
                    let preset = palette.preset;
                    terminal_theme_option(
                        &t,
                        palette,
                        theme_preset == preset,
                        Box::new({
                            let dialog = dialog.clone();
                            move |_, _, app| {
                                apply_dialog_settings(&dialog, app, move |settings| {
                                    settings.terminal_theme_preset = preset;
                                });
                            }
                        }),
                    )
                    .into_any_element()
                })
                .collect(),
        ))
        .child(setting_group(
            &t,
            "Font Size",
            vec![
                stepper_row(
                    &t,
                    "terminal-font-size",
                    "Terminal text size",
                    format!("{} px", font_size as u16).into(),
                    Box::new({
                        let dialog = dialog.clone();
                        move |_, _, app| {
                            apply_dialog_settings(&dialog, app, |settings| {
                                settings.terminal_font_size =
                                    settings.terminal_font_size.saturating_sub(1).max(10);
                            });
                        }
                    }),
                    Box::new({
                        let dialog = dialog.clone();
                        move |_, _, app| {
                            apply_dialog_settings(&dialog, app, |settings| {
                                settings.terminal_font_size =
                                    settings.terminal_font_size.saturating_add(1).min(24);
                            });
                        }
                    }),
                )
                .into_any_element(),
                div()
                    .mt(SP_2)
                    .px(SP_2)
                    .py(SP_2)
                    .rounded(RADIUS_SM)
                    .bg(preview_bg)
                    .border_1()
                    .border_color(t.color.border_subtle)
                    .child(
                        div()
                            .font(preview_font)
                            .text_size(preview_size)
                            .line_height(px(font_size * 1.38))
                            .text_color(preview_fg)
                            .child("ssh root@prod-box\nprintf \"!= => <= ===\"\ncargo check -p pier-ui-gpui"),
                    )
                    .into_any_element(),
            ],
        ))
        .child(setting_group(
            &t,
            "Appearance",
            vec![
                stepper_row(
                    &t,
                    "terminal-opacity",
                    "Background opacity",
                    format!("{opacity_pct}%").into(),
                    Box::new({
                        let dialog = dialog.clone();
                        move |_, _, app| {
                            apply_dialog_settings(&dialog, app, |settings| {
                                settings.terminal_opacity_pct =
                                    settings.terminal_opacity_pct.saturating_sub(5).max(30);
                            });
                        }
                    }),
                    Box::new({
                        let dialog = dialog.clone();
                        move |_, _, app| {
                            apply_dialog_settings(&dialog, app, |settings| {
                                settings.terminal_opacity_pct =
                                    settings.terminal_opacity_pct.saturating_add(5).min(100);
                            });
                        }
                    }),
                )
                .into_any_element(),
                toggle_row(
                    &t,
                    "Font Ligatures",
                    "Disable contextual alternates if you want raw terminal glyph rendering.",
                    font_ligatures,
                    Box::new({
                        let dialog = dialog.clone();
                        move |value, _, app| {
                            apply_dialog_settings(&dialog, app, move |settings| {
                                settings.terminal_font_ligatures = *value;
                            });
                        }
                    }),
                )
                .into_any_element(),
            ],
        ))
        .child(setting_group(
            &t,
            "Cursor",
            vec![
                choice_chip(
                    &t,
                    "Block",
                    cursor_style == TerminalCursorStyle::Block,
                    Box::new({
                        let dialog = dialog.clone();
                        move |_, _, app| {
                            apply_dialog_settings(&dialog, app, |settings| {
                                settings.terminal_cursor_style = TerminalCursorStyle::Block;
                            });
                        }
                    }),
                )
                .into_any_element(),
                choice_chip(
                    &t,
                    "Underline",
                    cursor_style == TerminalCursorStyle::Underline,
                    Box::new({
                        let dialog = dialog.clone();
                        move |_, _, app| {
                            apply_dialog_settings(&dialog, app, |settings| {
                                settings.terminal_cursor_style = TerminalCursorStyle::Underline;
                            });
                        }
                    }),
                )
                .into_any_element(),
                choice_chip(
                    &t,
                    "Bar",
                    cursor_style == TerminalCursorStyle::Bar,
                    Box::new({
                        let dialog = dialog.clone();
                        move |_, _, app| {
                            apply_dialog_settings(&dialog, app, |settings| {
                                settings.terminal_cursor_style = TerminalCursorStyle::Bar;
                            });
                        }
                    }),
                )
                .into_any_element(),
                toggle_row(
                    &t,
                    "Cursor Blink",
                    "Turn off if you prefer a stable insertion point.",
                    cursor_blink,
                    Box::new({
                        let dialog = dialog.clone();
                        move |value, _, app| {
                            apply_dialog_settings(&dialog, app, move |settings| {
                                settings.terminal_cursor_blink = *value;
                            });
                        }
                    }),
                )
                .into_any_element(),
            ],
        ))
    }

    fn render_shortcuts(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();

        let shortcuts = [
            ("New Tab", "Cmd/Ctrl+T"),
            ("Close Active Tab", "Cmd/Ctrl+Shift+W"),
            ("Open Settings", "Cmd/Ctrl+,"),
            ("Toggle Left Panel", "Cmd/Ctrl+\\"),
            ("Toggle Right Panel", "Cmd/Ctrl+Shift+\\"),
            ("Toggle Theme", "Cmd/Ctrl+Shift+L"),
            ("Copy Selection", "Cmd/Ctrl+C"),
            ("Paste", "Cmd/Ctrl+V"),
        ];

        let mut rows = Vec::with_capacity(shortcuts.len());
        for (label, shortcut) in shortcuts {
            rows.push(shortcut_row(&t, label, shortcut).into_any_element());
        }

        section_shell(
            &t,
            "Shortcuts",
            "These are the current bindings wired into the GPUI shell today.",
        )
        .children(rows)
    }
}

pub fn open(window: &mut Window, cx: &mut App) {
    log::info!("dialog: opening settings dialog");
    let view = cx.new(SettingsDialog::new);
    window.open_dialog(cx, move |dialog, _w, _app| {
        dialog
            .title("Settings")
            .w(px(760.0))
            .close_button(true)
            .overlay_closable(true)
            .keyboard(true)
            .child(view.clone())
    });
}

fn apply_dialog_settings(
    dialog: &WeakEntity<SettingsDialog>,
    app: &mut App,
    update: impl FnOnce(&mut AppSettings),
) {
    update_settings(app, update);
    let _ = dialog.update(app, |_, cx| cx.notify());
}

fn section_shell(
    t: &crate::theme::Theme,
    title: &'static str,
    subtitle: &'static str,
) -> gpui_component::scroll::Scrollable<gpui::Div> {
    div()
        .h_full()
        .overflow_y_scrollbar()
        .px(SP_4)
        .py(SP_4)
        .flex()
        .flex_col()
        .gap(SP_4)
        .child(
            div()
                .child(
                    div()
                        .text_size(SIZE_H2)
                        .font_weight(WEIGHT_EMPHASIS)
                        .text_color(t.color.text_primary)
                        .child(title),
                )
                .child(
                    div()
                        .pt(px(4.0))
                        .text_size(SIZE_SMALL)
                        .text_color(t.color.text_tertiary)
                        .child(subtitle),
                ),
        )
}

fn setting_group(
    t: &crate::theme::Theme,
    title: &'static str,
    children: Vec<gpui::AnyElement>,
) -> impl IntoElement {
    div()
        .p(SP_3)
        .flex()
        .flex_col()
        .gap(SP_2)
        .rounded(px(6.0))
        .bg(t.color.bg_panel)
        .border_1()
        .border_color(t.color.border_subtle)
        .child(
            div()
                .text_size(SIZE_BODY)
                .font_weight(WEIGHT_MEDIUM)
                .text_color(t.color.text_primary)
                .child(title),
        )
        .children(children)
}

fn choice_chip(
    t: &crate::theme::Theme,
    label: &str,
    selected: bool,
    on_click: Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
) -> impl IntoElement {
    let id: SharedString = format!("settings-chip-{label}").into();
    div()
        .id(gpui::ElementId::Name(id))
        .px(SP_2)
        .py(SP_1_5)
        .rounded(RADIUS_SM)
        .border_1()
        .border_color(if selected {
            t.color.accent
        } else {
            t.color.border_subtle
        })
        .bg(if selected {
            t.color.accent_subtle
        } else {
            t.color.bg_surface
        })
        .text_color(if selected {
            t.color.accent
        } else {
            t.color.text_secondary
        })
        .cursor_pointer()
        .hover(|style| style.bg(t.color.bg_hover))
        .on_click(on_click)
        .child(
            div()
                .text_size(SIZE_CAPTION)
                .font_weight(WEIGHT_MEDIUM)
                .child(SharedString::from(label.to_string())),
        )
}

fn stepper_row(
    t: &crate::theme::Theme,
    id_prefix: &'static str,
    label: &'static str,
    value: SharedString,
    on_minus: Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
    on_plus: Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .child(
            div().flex_1().min_w(px(0.0)).child(
                div()
                    .text_size(SIZE_BODY)
                    .text_color(t.color.text_secondary)
                    .child(label),
            ),
        )
        .child(icon_step_button(t, id_prefix, "-", on_minus))
        .child(
            div()
                .min_w(px(58.0))
                .text_size(SIZE_CAPTION)
                .font_weight(WEIGHT_MEDIUM)
                .text_color(t.color.text_primary)
                .child(value),
        )
        .child(icon_step_button(t, id_prefix, "+", on_plus))
}

fn icon_step_button(
    t: &crate::theme::Theme,
    id_prefix: &'static str,
    label: &'static str,
    on_click: Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
) -> impl IntoElement {
    let id: SharedString = format!("settings-step-{id_prefix}-{label}").into();
    div()
        .id(gpui::ElementId::Name(id))
        .w(px(24.0))
        .h(px(24.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(RADIUS_SM)
        .bg(t.color.bg_surface)
        .border_1()
        .border_color(t.color.border_subtle)
        .text_color(t.color.text_secondary)
        .cursor_pointer()
        .hover(|style| style.bg(t.color.bg_hover).text_color(t.color.text_primary))
        .on_click(on_click)
        .child(
            div()
                .text_size(SIZE_BODY)
                .font_weight(WEIGHT_MEDIUM)
                .child(label),
        )
}

fn toggle_row(
    t: &crate::theme::Theme,
    title: &'static str,
    description: &'static str,
    checked: bool,
    on_click: Box<dyn Fn(&bool, &mut Window, &mut App) + 'static>,
) -> impl IntoElement {
    div()
        .mt(SP_2)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .child(
                    div()
                        .text_size(SIZE_BODY)
                        .font_weight(WEIGHT_MEDIUM)
                        .text_color(t.color.text_primary)
                        .child(title),
                )
                .child(
                    div()
                        .pt(px(2.0))
                        .text_size(SIZE_SMALL)
                        .text_color(t.color.text_tertiary)
                        .child(description),
                ),
        )
        .child(
            gpui_component::switch::Switch::new(title)
                .checked(checked)
                .on_click(on_click),
        )
}

fn shortcut_row(
    t: &crate::theme::Theme,
    label: &'static str,
    shortcut: &'static str,
) -> impl IntoElement {
    div()
        .px(SP_2)
        .py(SP_2)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .rounded(RADIUS_SM)
        .bg(t.color.bg_panel)
        .border_1()
        .border_color(t.color.border_subtle)
        .child(
            div()
                .flex_1()
                .text_size(SIZE_BODY)
                .text_color(t.color.text_secondary)
                .child(label),
        )
        .child(
            div()
                .px(SP_2)
                .py(SP_1)
                .rounded(RADIUS_SM)
                .bg(t.color.bg_canvas)
                .border_1()
                .border_color(t.color.border_subtle)
                .font_family(t.font_mono.clone())
                .text_size(SIZE_CAPTION)
                .text_color(t.color.text_primary)
                .child(shortcut),
        )
}

fn terminal_theme_option(
    t: &crate::theme::Theme,
    palette: crate::theme::terminal::TerminalPalette,
    selected: bool,
    on_click: Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
) -> impl IntoElement {
    let id: SharedString = format!("settings-terminal-theme-{:?}", palette.preset).into();

    div()
        .id(gpui::ElementId::Name(id))
        .px(SP_2)
        .py(SP_2)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .rounded(RADIUS_SM)
        .border_1()
        .border_color(if selected {
            t.color.accent
        } else {
            t.color.border_subtle
        })
        .bg(if selected {
            t.color.accent_subtle
        } else {
            t.color.bg_panel
        })
        .cursor_pointer()
        .hover(|style| style.bg(t.color.bg_hover))
        .on_click(on_click)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(4.0))
                .child(color_dot(palette.background_hex))
                .child(color_dot(palette.foreground_hex))
                .child(color_dot(palette.cursor_bg_hex))
                .child(color_dot(palette.ansi_16[4]))
                .child(color_dot(palette.ansi_16[2]))
                .child(color_dot(palette.ansi_16[1])),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .child(
                    div()
                        .text_size(SIZE_BODY)
                        .font_weight(WEIGHT_MEDIUM)
                        .text_color(if selected {
                            t.color.accent
                        } else {
                            t.color.text_primary
                        })
                        .child(palette.name),
                )
                .child(
                    div()
                        .pt(px(2.0))
                        .text_size(SIZE_SMALL)
                        .text_color(t.color.text_tertiary)
                        .child(match palette.preset {
                            TerminalThemePreset::DefaultDark
                            | TerminalThemePreset::DefaultLight => "Pier default shell palette",
                            TerminalThemePreset::SolarizedDark => {
                                "Low-contrast palette for long sessions"
                            }
                            TerminalThemePreset::Dracula => "High-contrast violet-heavy palette",
                            TerminalThemePreset::Monokai => "Warm palette with vivid syntax colors",
                            TerminalThemePreset::Nord => "Cool muted palette with softer contrast",
                        }),
                ),
        )
}

fn color_dot(hex: u32) -> impl IntoElement {
    div()
        .w(px(12.0))
        .h(px(12.0))
        .rounded(px(999.0))
        .bg(terminal_hex_color(hex))
}
