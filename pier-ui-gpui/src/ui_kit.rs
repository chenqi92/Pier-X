use gpui::{App, Hsla, Rgba};
use gpui_component::theme::{Theme as ComponentTheme, ThemeColor, ThemeMode as ComponentThemeMode};

use crate::theme::{
    self,
    radius::{RADIUS_MD, RADIUS_NONE, RADIUS_SM},
    spacing::SP_1,
    typography::{SIZE_BODY, SIZE_MONO_CODE},
    ColorSet, ThemeMode,
};

pub fn init(cx: &mut App) {
    gpui_component::init(cx);
}

pub fn sync_theme(cx: &mut App) {
    let pier = theme::theme(cx).clone();
    let component = ComponentTheme::global_mut(cx);

    component.mode = match pier.mode {
        ThemeMode::Dark => ComponentThemeMode::Dark,
        ThemeMode::Light => ComponentThemeMode::Light,
    };
    component.colors = map_colors(pier.color, pier.mode);
    component.font_family = pier.font_ui;
    component.font_size = SIZE_BODY;
    component.mono_font_family = pier.font_mono;
    component.mono_font_size = SIZE_MONO_CODE;
    component.radius = RADIUS_SM;
    component.radius_lg = RADIUS_MD;
    component.shadow = false;
    component.tile_grid_size = SP_1;
    component.tile_radius = RADIUS_NONE;

    cx.refresh_windows();
}

fn map_colors(colors: ColorSet, mode: ThemeMode) -> ThemeColor {
    let mut mapped = match mode {
        ThemeMode::Dark => *ThemeColor::dark(),
        ThemeMode::Light => *ThemeColor::light(),
    };

    mapped.accent = hsla(colors.accent_muted);
    mapped.accent_foreground = hsla(colors.accent);
    mapped.background = hsla(colors.bg_canvas);
    mapped.border = hsla(colors.border_default);
    mapped.caret = hsla(colors.accent);
    mapped.description_list_label = hsla(colors.bg_panel);
    mapped.description_list_label_foreground = hsla(colors.text_secondary);
    mapped.foreground = hsla(colors.text_primary);
    mapped.group_box = hsla(colors.bg_surface);
    mapped.group_box_foreground = hsla(colors.text_secondary);
    mapped.info = hsla(colors.status_info);
    mapped.info_active = hsla(colors.status_info);
    mapped.info_foreground = hsla(colors.text_inverse);
    mapped.info_hover = hsla(colors.status_info);
    mapped.input = hsla(colors.border_default);
    mapped.link = hsla(colors.accent);
    mapped.link_active = hsla(colors.accent);
    mapped.link_hover = hsla(colors.accent_hover);
    mapped.list = hsla(colors.bg_surface);
    mapped.list_active = hsla(colors.bg_selected);
    mapped.list_active_border = hsla(colors.border_focus);
    mapped.list_even = hsla(colors.bg_panel);
    mapped.list_head = hsla(colors.bg_panel);
    mapped.list_hover = hsla(colors.bg_hover);
    mapped.muted = hsla(colors.bg_panel);
    mapped.muted_foreground = hsla(colors.text_tertiary);
    mapped.popover = hsla(colors.bg_elevated);
    mapped.popover_foreground = hsla(colors.text_primary);
    mapped.primary = hsla(colors.accent);
    mapped.primary_active = hsla(colors.accent);
    mapped.primary_foreground = hsla(colors.text_inverse);
    mapped.primary_hover = hsla(colors.accent_hover);
    mapped.progress_bar = hsla(colors.accent);
    mapped.ring = hsla(colors.border_focus);
    mapped.scrollbar = hsla(Rgba::default());
    mapped.scrollbar_thumb = hsla(Rgba {
        a: 0.28,
        ..colors.text_tertiary
    });
    mapped.scrollbar_thumb_hover = hsla(Rgba {
        a: 0.42,
        ..colors.text_secondary
    });
    mapped.secondary = hsla(colors.bg_surface);
    mapped.secondary_active = hsla(colors.bg_active);
    mapped.secondary_foreground = hsla(colors.text_primary);
    mapped.secondary_hover = hsla(colors.bg_hover);
    mapped.selection = hsla(colors.bg_selected);
    mapped.sidebar = hsla(colors.bg_panel);
    mapped.sidebar_accent = hsla(colors.bg_hover);
    mapped.sidebar_accent_foreground = hsla(colors.text_primary);
    mapped.sidebar_border = hsla(colors.border_subtle);
    mapped.sidebar_foreground = hsla(colors.text_primary);
    mapped.sidebar_primary = hsla(colors.accent);
    mapped.sidebar_primary_foreground = hsla(colors.text_inverse);
    mapped.skeleton = hsla(colors.bg_panel);
    mapped.success = hsla(colors.status_success);
    mapped.success_active = hsla(colors.status_success);
    mapped.success_foreground = hsla(colors.text_inverse);
    mapped.success_hover = hsla(colors.status_success);
    // Switch OFF track — `bg_panel` was invisible against `bg_canvas`
    // and let the thumb read as a solid dark pill (reported as "switch
    // colors wrong in light mode"). `border_strong` gives 18 % black
    // on light, 14 % white on dark — both read as a visible neutral
    // track similar to macOS-native switches.
    mapped.switch = hsla(colors.border_strong);
    // Thumb always white — macOS native behavior. `text_primary` was
    // near-black on light mode, which turned OFF switches into a solid
    // black knob floating over the dialog.
    mapped.switch_thumb = hsla(Rgba {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    });
    mapped.tab = hsla(colors.bg_panel);
    mapped.tab_active = hsla(colors.bg_surface);
    mapped.tab_active_foreground = hsla(colors.text_primary);
    mapped.tab_bar = hsla(colors.bg_panel);
    mapped.tab_bar_segmented = hsla(colors.bg_surface);
    mapped.tab_foreground = hsla(colors.text_secondary);
    mapped.table = hsla(colors.bg_surface);
    mapped.table_active = hsla(colors.bg_selected);
    mapped.table_active_border = hsla(colors.border_focus);
    mapped.table_even = hsla(colors.bg_panel);
    mapped.table_head = hsla(colors.bg_panel);
    mapped.table_head_foreground = hsla(colors.text_secondary);
    mapped.table_hover = hsla(colors.bg_hover);
    mapped.table_row_border = hsla(colors.border_subtle);
    mapped.title_bar = hsla(colors.bg_panel);
    mapped.title_bar_border = hsla(colors.border_subtle);
    mapped.warning = hsla(colors.status_warning);
    mapped.warning_active = hsla(colors.status_warning);
    mapped.warning_foreground = hsla(colors.text_inverse);
    mapped.warning_hover = hsla(colors.status_warning);
    mapped.window_border = hsla(colors.border_subtle);

    mapped.red = hsla(colors.status_error);
    mapped.red_light = hsla(colors.status_error);
    mapped.green = hsla(colors.status_success);
    mapped.green_light = hsla(colors.status_success);
    mapped.blue = hsla(colors.accent);
    mapped.blue_light = hsla(colors.accent_hover);
    mapped.yellow = hsla(colors.status_warning);
    mapped.yellow_light = hsla(colors.status_warning);

    mapped.danger = hsla(colors.status_error);
    mapped.danger_active = hsla(colors.status_error);
    mapped.danger_foreground = hsla(colors.text_inverse);
    mapped.danger_hover = hsla(colors.status_error);

    mapped
}

fn hsla(color: Rgba) -> Hsla {
    color.into()
}
