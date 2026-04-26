//! Mirrored pack schema. Same shape as
//! `pier_core::terminal::library::CommandPack`, but the importer
//! treats it as a strictly local data type so we never have to
//! depend on pier-core having `Deserialize` derives on its types.
//!
//! Field order matters for the JSON output to read consistently
//! across packs — the diff between two runs of the importer should
//! reflect content changes, not field-order shuffles.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommandPack {
    pub schema_version: u32,
    pub command: String,
    pub tool_version: String,
    pub source: String,
    pub import_method: String,
    pub import_date: String,
    pub subcommands: Vec<SubcommandEntry>,
    pub options: Vec<OptionEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubcommandEntry {
    pub name: String,
    /// `BTreeMap` for stable JSON key order across runs.
    #[serde(default)]
    pub i18n: BTreeMap<String, String>,
    #[serde(default)]
    pub options: Vec<OptionEntry>,
    #[serde(default)]
    pub subcommands: Vec<SubcommandEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OptionEntry {
    pub flag: String,
    #[serde(default)]
    pub i18n: BTreeMap<String, String>,
}

impl SubcommandEntry {
    /// Build a single-locale entry. The importer always populates
    /// `en` initially; a later locale-overlay pass adds zh-CN.
    pub fn with_en(name: impl Into<String>, description: impl Into<String>) -> Self {
        let mut i18n = BTreeMap::new();
        i18n.insert("en".to_string(), description.into());
        Self {
            name: name.into(),
            i18n,
            options: Vec::new(),
            subcommands: Vec::new(),
        }
    }
}

impl OptionEntry {
    pub fn with_en(flag: impl Into<String>, description: impl Into<String>) -> Self {
        let mut i18n = BTreeMap::new();
        i18n.insert("en".to_string(), description.into());
        Self {
            flag: flag.into(),
            i18n,
        }
    }
}
