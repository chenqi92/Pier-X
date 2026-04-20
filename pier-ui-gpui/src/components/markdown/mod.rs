//! Markdown preview atoms.
//!
//! The right-panel Markdown reader is the only current caller, but
//! code blocks, data tables, and accent-ruled blockquotes are visual
//! primitives worth owning here (CLAUDE.md §Rule 2) — views compose
//! them rather than inline the chrome.

pub mod blockquote;
pub mod code_block;
pub mod data_table;

pub use blockquote::MarkdownBlockquote;
pub use code_block::MarkdownCodeBlock;
pub use data_table::{MarkdownDataTable, MarkdownTableAlign};
