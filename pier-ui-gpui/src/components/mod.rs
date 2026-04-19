// Components re-exported here are the public face of the design
// system — some will sit unused for a while as views migrate onto
// them. Silence `dead_code`/`unused_imports` at module level rather
// than per-re-export; a compiler warning here is noise, not a real
// problem.
#![allow(unused_imports, dead_code)]

pub mod assist_strip;
pub mod button;
pub mod card;
pub mod icon_badge;
pub mod icon_button;
pub mod meta_line;
pub mod page_header;
pub mod page_toolbar;
pub mod section_label;
pub mod status_pill;
pub mod tabs;
pub(crate) mod terminal_grid;
pub mod text;

pub use assist_strip::AssistStrip;
pub use button::{Button, ButtonSize, ButtonVariant};
pub use card::Card;
pub use icon_badge::IconBadge;
pub use icon_button::{IconButton, IconButtonSize, IconButtonVariant};
pub use meta_line::MetaLine;
pub use page_header::{HeaderSize, PageHeader};
pub use page_toolbar::PageToolbar;
pub use section_label::SectionLabel;
pub use status_pill::{StatusKind, StatusPill};
pub use tabs::{TabItem, Tabs};
