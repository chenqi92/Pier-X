#![allow(dead_code)]
//! Layout height & size tokens.
//!
//! Typography and spacing are token-ified elsewhere, but component
//! heights (toolbar rails, button rows, pill capsules, icon squares)
//! used to live as `px(...)` literals buried inside each component
//! file. That made a "UI font just got bigger, bump every row" change
//! impossible without grepping across six files.
//!
//! Keep literals in ONE place here; components import the constant.
//! Per SKILL.md §9 the values below match the design spec — component
//! files may reference them freely, and view files MUST compose
//! components rather than apply heights themselves (see CLAUDE.md §Rule 1).

use gpui::{px, Pixels};

// Shell rails.
pub const TOOLBAR_H: Pixels = px(32.0);
pub const STATUSBAR_H: Pixels = px(22.0);

// Page grammar containers.
pub const PAGEHEADER_H: Pixels = px(42.0);
pub const ASSIST_STRIP_H: Pixels = px(30.0);

// Buttons & interactive rows.
pub const BUTTON_XS_H: Pixels = px(18.0);
pub const BUTTON_SM_H: Pixels = px(22.0);
pub const BUTTON_MD_H: Pixels = px(28.0);

// Status pill capsule — matches SKILL.md §9.
pub const PILL_H: Pixels = px(18.0);
pub const PILL_DOT: Pixels = px(6.0);

// List rows / tab rows.
pub const ROW_SM_H: Pixels = px(24.0);
pub const ROW_MD_H: Pixels = px(28.0);

// Inspector-grammar primitives (right-panel mode bodies).
// PropertyRow tight = 22px (label:value); InspectorSection header =
// 28px (small section title bar); DataCell = 56px two-line stat tile.
// Kept separate from the generic list rows so the inspector grid feels
// denser than a navigable list without stealing ROW_SM/ROW_MD values.
pub const INSPECTOR_ROW_H: Pixels = px(22.0);
pub const INSPECTOR_HEADER_H: Pixels = px(28.0);
pub const INSPECTOR_CELL_H: Pixels = px(56.0);

// Tab pill — a hair taller than ROW_SM so the pill doesn't kiss the
// bottom rule, plus its own inline-glyph size so the tab icon feels
// deliberately smaller than the adjacent label text.
pub const TAB_PILL_H: Pixels = px(22.0);
pub const TAB_GLYPH: Pixels = px(13.0);

// Icon sizes (inside buttons, labels, rows).
pub const ICON_SM: Pixels = px(14.0);
pub const ICON_MD: Pixels = px(16.0);

// Inline glyph sizes (for plain icons inside rows / chips, distinct
// from button icons). GLYPH_* is the 10/11/12/14 px bucket that shows
// up in tab close buttons, tree-row caret arrows, and pill dots.
pub const GLYPH_XS: Pixels = px(10.0);
pub const GLYPH_2XS: Pixels = px(11.0);
pub const GLYPH_SM: Pixels = px(12.0);
pub const GLYPH_MD: Pixels = px(14.0);
