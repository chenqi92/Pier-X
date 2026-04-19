//! Extended UI widgets — sibling of `components/`.
//!
//! `components/` holds the core, small, heavily-reused atoms
//! (Button, Card, StatusPill, text helpers, Tabs). `widgets/`
//! holds higher-level compositions that are too business-specific
//! to live next to those atoms but too common to duplicate across
//! views:
//!
//!   * `SegmentedControl` — iOS / macOS Settings-style segmented
//!     picker used by the Docker panel to switch between
//!     containers / images / volumes.
//!
//! Any new "molecule" that composes several `components/` atoms
//! with business-specific styling goes here. Keep `components/`
//! lean and theme-only; do the feature UX wiring in `widgets/`.

pub mod segmented;
pub mod settings_section;

pub use segmented::{SegmentedControl, SegmentedItem};
pub use settings_section::SettingsSection;
