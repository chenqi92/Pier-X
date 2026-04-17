#![allow(dead_code)]

use gpui::{hsla, point, px, BoxShadow};

fn shadow(x: f32, y: f32, blur: f32, _spread: f32, alpha: f32) -> BoxShadow {
    BoxShadow {
        color: hsla(0.0, 0.0, 0.0, alpha),
        offset: point(px(x), px(y)),
        blur_radius: px(blur),
        spread_radius: px(0.0),
    }
}

pub fn soft() -> Vec<BoxShadow> {
    vec![
        shadow(0.0, 1.0, 2.0, 0.0, 0.20),
        shadow(0.0, 2.0, 6.0, 0.0, 0.16),
    ]
}

pub fn popover() -> Vec<BoxShadow> {
    vec![
        shadow(0.0, 8.0, 24.0, 0.0, 0.32),
        shadow(0.0, 2.0, 8.0, 0.0, 0.24),
    ]
}

pub fn modal() -> Vec<BoxShadow> {
    vec![
        shadow(0.0, 24.0, 64.0, 0.0, 0.48),
        shadow(0.0, 8.0, 24.0, 0.0, 0.32),
    ]
}
