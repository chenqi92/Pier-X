#![allow(dead_code)]

use gpui::{px, FontWeight, Pixels};

// Type ramp per SKILL.md §3.2 — 12px UI baseline to match SwiftUI
// density (Pier uses 11–12pt in mixed roles). Previous ramp had a
// 13px baseline which read one step too heavy versus the reference app.
pub const SIZE_DISPLAY: Pixels = px(28.0);
pub const SIZE_H1: Pixels = px(20.0);
pub const SIZE_H2: Pixels = px(16.0);
pub const SIZE_H3: Pixels = px(14.0);
pub const SIZE_BODY_LARGE: Pixels = px(13.0);
pub const SIZE_BODY: Pixels = px(12.0);
pub const SIZE_UI_LABEL: Pixels = px(12.0);
pub const SIZE_CAPTION: Pixels = px(11.0);
pub const SIZE_SMALL: Pixels = px(10.0);
pub const SIZE_MONO_CODE: Pixels = px(12.0);
pub const SIZE_MONO_SMALL: Pixels = px(11.0);

pub const WEIGHT_REGULAR: FontWeight = FontWeight(400.0);
pub const WEIGHT_MEDIUM: FontWeight = FontWeight(510.0);
pub const WEIGHT_EMPHASIS: FontWeight = FontWeight(590.0);
