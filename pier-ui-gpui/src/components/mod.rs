// Components re-exported here are the public face of the design
// system — some will sit unused for a while as views migrate onto
// them. Silence `dead_code`/`unused_imports` at module level rather
// than per-re-export; a compiler warning here is noise, not a real
// problem.
#![allow(unused_imports, dead_code)]

pub mod assist_strip;
pub mod button;
pub mod card;
pub mod commit_graph;
pub mod context_menu;
pub mod data_cell;
pub mod dropdown;
pub mod form_field;
pub mod icon_badge;
pub mod icon_button;
pub mod inline_input;
pub mod inspector_section;
pub mod meta_line;
pub mod page_header;
pub mod page_toolbar;
pub mod pill_cluster;
pub mod property_row;
pub mod section_label;
pub mod separator;
pub mod setting_row;
pub mod status_pill;
pub mod tabs;
pub(crate) mod terminal_grid;
pub mod text;
pub mod toggle_row;
pub mod transfer_toast;

pub use assist_strip::AssistStrip;
pub use button::{Button, ButtonSize, ButtonVariant};
pub use card::Card;
pub use commit_graph::{
    compute_graph_col_width, graph_row_canvas, is_head_row, palette_color, DOT_RADIUS, LANE_WIDTH,
    ROW_HEIGHT,
};
pub use context_menu::{ContextMenu, ContextMenuItem};
pub use data_cell::{data_cell_row, DataCell, DataTone};
pub use dropdown::{Dropdown, DropdownOption, DropdownSize};
pub use form_field::{FormField, FormSection};
pub use icon_badge::IconBadge;
pub use icon_button::{IconButton, IconButtonSize, IconButtonVariant};
pub use inline_input::{InlineInput, InlineInputTone};
pub use inspector_section::InspectorSection;
pub use meta_line::MetaLine;
pub use page_header::{HeaderSize, PageHeader};
pub use page_toolbar::PageToolbar;
pub use pill_cluster::PillCluster;
pub use property_row::{PropertyRow, PropertyRowVariant};
pub use section_label::SectionLabel;
pub use separator::{Separator, SeparatorAxis, SeparatorVariant};
pub use setting_row::SettingRow;
pub use status_pill::{StatusKind, StatusPill};
pub use tabs::{TabItem, Tabs, TabsVariant};
pub use toggle_row::ToggleRow;
pub use transfer_toast::TransferToast;
