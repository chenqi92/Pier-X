//! Command library — bundled + user-supplied subcommand / flag
//! descriptions for the smart-mode completion popover.
//!
//! ## Why this exists
//!
//! The base `complete()` engine in [`super::completions`] only knows
//! about PATH binaries and files in cwd — when the user types
//! `docker ` the second word lands in argument position and gets a
//! file listing. That's useless: the user wants to see `build`,
//! `pull`, `compose`, etc. with descriptions of what each does.
//!
//! This module loads structured packs that **describe** known
//! commands: their subcommands, options, and a localized
//! description string for each. The popover layer joins the engine's
//! candidates with whatever the library provides for the current
//! command, falling back gracefully when nothing is known.
//!
//! ## Data shape
//!
//! Each pack is a JSON file with this top level:
//!
//! ```jsonc
//! {
//!   "schema_version": 1,
//!   "command": "docker",
//!   "tool_version": "27.0",        // upstream version when imported
//!   "source": "auto-imported",     // "auto-imported" | "user" | "bundled-seed"
//!   "import_method": "completion-zsh", // how it was extracted
//!   "import_date": "2026-04-26",
//!   "subcommands": [
//!     {
//!       "name": "build",
//!       "i18n": {
//!         "en": "Build an image from a Dockerfile",
//!         "zh-CN": "从 Dockerfile 构建镜像"
//!       },
//!       "options": [ /* same shape as top-level options */ ]
//!     }
//!   ],
//!   "options": [
//!     {
//!       "flag": "-H, --host",
//!       "i18n": { "en": "Daemon socket(s) to connect to" }
//!     }
//!   ]
//! }
//! ```
//!
//! ## Bundled vs user
//!
//! [`Library::bundled()`] decodes a small set of packs embedded at
//! compile time — enough so a fresh install of Pier-X already gives
//! useful suggestions for the most common CLIs. [`Library::merge_user`]
//! layers user-supplied packs on top (added later, alongside the
//! disk loader in `pier-core/src/completions/loader.rs`).
//!
//! Lookup is **stateless** — every call to [`Library::lookup`] hashes
//! the command name and walks subcommands at most twice. There's no
//! global state, no init step.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Current pack schema version. Bumped when the JSON shape changes
/// in a way that requires migration. Loaders refuse packs whose
/// `schema_version` is greater than this constant — older Pier-X
/// shouldn't crash on a future-format pack pushed by online update.
pub const SCHEMA_VERSION: u32 = 1;

/// One command's full description: top-level flags + subcommands.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CommandPack {
    /// Pack format version. See [`SCHEMA_VERSION`].
    pub schema_version: u32,
    /// The command name as it appears on the command line — first
    /// word of the input (e.g. `"docker"`, `"git"`).
    pub command: String,
    /// Upstream tool version string captured at import time.
    /// Empty when not parseable. Diagnostic only — the lookup path
    /// doesn't gate on this.
    #[serde(default)]
    pub tool_version: String,
    /// `"auto-imported"`, `"user"`, or `"bundled-seed"`. Surfaced in
    /// the Settings library list so the user can tell where each
    /// row came from.
    #[serde(default)]
    pub source: String,
    /// `"completion-zsh"`, `"man"`, `"help"`, or `"hand-curated"`.
    /// Used by the importer's scoring step + Settings panel.
    #[serde(default)]
    pub import_method: String,
    /// ISO-8601 date (YYYY-MM-DD) when the pack was generated.
    #[serde(default)]
    pub import_date: String,
    /// Subcommands as they appear at the second-word position.
    /// Empty for single-level CLIs like `ls` / `grep`.
    #[serde(default)]
    pub subcommands: Vec<SubcommandEntry>,
    /// Top-level option flags applicable to the bare command.
    #[serde(default)]
    pub options: Vec<OptionEntry>,
}

/// One subcommand row. The popover shows the `name` + the localized
/// description and lets the user drill into nested options.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubcommandEntry {
    /// The literal token typed at the command line (e.g. `"build"`).
    pub name: String,
    /// Locale → description string. Resolution prefers the user's
    /// active locale, then `"en"`, then any other available locale.
    /// Missing entirely → no description shown.
    #[serde(default)]
    pub i18n: HashMap<String, String>,
    /// Subcommand-specific flags. `--help` is implicit and not
    /// listed here; importers should drop it.
    #[serde(default)]
    pub options: Vec<OptionEntry>,
    /// Nested subcommands (e.g. `git remote add` — `add` is a
    /// nested subcommand under `remote`). Loaders walk the chain.
    #[serde(default)]
    pub subcommands: Vec<SubcommandEntry>,
}

/// One option flag row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionEntry {
    /// As it appears in the help output, e.g. `"-t, --tag"` or
    /// `"--quiet"`. The importer normalizes to "shortest-form
    /// first, longest-form last, comma-separated".
    pub flag: String,
    /// Locale → description, same rules as
    /// [`SubcommandEntry::i18n`].
    #[serde(default)]
    pub i18n: HashMap<String, String>,
}

/// In-memory command library. Resolved once at startup; the
/// terminal completer borrows it during every Tab.
#[derive(Debug, Clone, Default)]
pub struct Library {
    /// Command-name → pack. Multiple packs for the same command
    /// (bundled + user override) collapse before insertion: user
    /// wins.
    by_command: HashMap<String, CommandPack>,
}

impl Library {
    /// Empty library — useful for tests.
    pub fn empty() -> Self {
        Self {
            by_command: HashMap::new(),
        }
    }

    /// Library seeded with whatever ships inside the binary. The
    /// content list is in [`bundled_seeds`] below — keep it small,
    /// the goal is "the obvious commands feel rich"; everything else
    /// resolves at runtime via the importer's dynamic path.
    pub fn bundled() -> Self {
        let mut lib = Self::empty();
        for raw in bundled_seeds() {
            match serde_json::from_str::<CommandPack>(raw) {
                Ok(pack) if pack.schema_version <= SCHEMA_VERSION => {
                    lib.insert(pack);
                }
                Ok(pack) => {
                    log::warn!(
                        "skipping bundled pack {:?} — schema_version {} > {}",
                        pack.command,
                        pack.schema_version,
                        SCHEMA_VERSION
                    );
                }
                Err(e) => {
                    log::warn!("malformed bundled pack: {e}");
                }
            }
        }
        lib
    }

    /// Merge `pack` into the library, replacing any existing entry
    /// for the same command. Used by the user-pack loader.
    pub fn insert(&mut self, pack: CommandPack) {
        self.by_command.insert(pack.command.clone(), pack);
    }

    /// Number of commands the library knows about. Surfaced in
    /// the Settings UI as `"已安装 N 条命令"`.
    pub fn len(&self) -> usize {
        self.by_command.len()
    }

    /// True when no packs are loaded.
    pub fn is_empty(&self) -> bool {
        self.by_command.is_empty()
    }

    /// All command names the library has packs for, sorted
    /// alphabetically. Used by the Settings UI.
    pub fn commands(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.by_command.keys().map(String::as_str).collect();
        names.sort_unstable();
        names
    }

    /// Borrow the pack for `cmd`, if any.
    pub fn lookup(&self, cmd: &str) -> Option<&CommandPack> {
        self.by_command.get(cmd)
    }

    /// Load every `*.json` file in `dir` and merge it into the
    /// library. Returns the count of successfully loaded packs.
    /// Malformed files are logged and skipped — one bad pack
    /// shouldn't take the whole load with it.
    ///
    /// `dir` is allowed to not exist (returns 0). The loader does
    /// **not** create the directory on its own; the Settings flow
    /// owns that.
    pub fn merge_dir(&mut self, dir: &Path) -> usize {
        let mut loaded = 0usize;
        let entries = match std::fs::read_dir(dir) {
            Ok(it) => it,
            Err(_) => return 0,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let body = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("user pack {:?}: read failed: {e}", path);
                    continue;
                }
            };
            let pack: CommandPack = match serde_json::from_str(&body) {
                Ok(p) => p,
                Err(e) => {
                    log::warn!("user pack {:?}: parse failed: {e}", path);
                    continue;
                }
            };
            if pack.schema_version > SCHEMA_VERSION {
                log::warn!(
                    "user pack {:?}: schema_version {} > {}; skipping",
                    path,
                    pack.schema_version,
                    SCHEMA_VERSION
                );
                continue;
            }
            self.insert(pack);
            loaded += 1;
        }
        loaded
    }

    /// Build the full library by stacking bundled + user packs.
    /// User packs win when both define the same command — that's
    /// the override path users use to fix or extend a bundled
    /// pack without rebuilding Pier-X.
    pub fn from_bundled_and_dir(user_dir: Option<&Path>) -> Self {
        let mut lib = Self::bundled();
        if let Some(dir) = user_dir {
            let n = lib.merge_dir(dir);
            if n > 0 {
                log::info!("loaded {n} user completion pack(s) from {:?}", dir);
            }
        }
        lib
    }

    /// Iterator-friendly view: `(command_name, pack)` pairs sorted
    /// alphabetically. Used by the Settings UI to render the list
    /// of installed packs.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &CommandPack)> {
        let mut pairs: Vec<(&str, &CommandPack)> = self
            .by_command
            .iter()
            .map(|(k, v)| (k.as_str(), v))
            .collect();
        pairs.sort_by_key(|(k, _)| *k);
        pairs.into_iter()
    }

    /// Resolve `i18n` for the user's locale. Resolution order:
    ///   1. Exact match on the requested locale (`"zh-CN"`).
    ///   2. Language root if the locale was a region tag
    ///      (`"zh-CN"` → try `"zh"`).
    ///   3. Any region under the requested language root
    ///      (`"zh"` → match `"zh-CN"` / `"zh-TW"`, first wins).
    ///   4. `"en"` fallback.
    ///   5. First value in the map.
    ///   6. Empty string.
    ///
    /// Both subcommands and options use the same chain so the
    /// description shown in the popover stays consistent across row
    /// kinds.
    pub fn pick_locale<'a>(i18n: &'a HashMap<String, String>, locale: &str) -> &'a str {
        if let Some(s) = i18n.get(locale) {
            return s;
        }
        if let Some((root, _)) = locale.split_once('-') {
            if let Some(s) = i18n.get(root) {
                return s;
            }
        }
        // User asked for a bare language tag (e.g. "zh") but the
        // pack only has region-qualified entries (e.g. "zh-CN").
        // Pick the first one whose region tag starts with our
        // language. Stable iteration isn't critical — the bundled
        // packs only ever ship one region per language today.
        let prefix = format!("{}-", locale);
        if let Some((_, s)) = i18n.iter().find(|(k, _)| k.starts_with(&prefix)) {
            return s;
        }
        if let Some(s) = i18n.get("en") {
            return s;
        }
        i18n.values().next().map(String::as_str).unwrap_or("")
    }
}

/// JSON blobs embedded at compile time. One per command. Keeping
/// them as raw `&str` avoids a build-time dependency on a JSON
/// codegen crate — the runtime parses them on first call to
/// [`Library::bundled`].
///
/// Add new bundled commands here. The first 10 covers the bulk of
/// what users type; everything else is fetched at runtime.
fn bundled_seeds() -> &'static [&'static str] {
    &[
        include_str!("../../resources/completions/docker.json"),
        include_str!("../../resources/completions/git.json"),
        include_str!("../../resources/completions/kubectl.json"),
        include_str!("../../resources/completions/npm.json"),
        include_str!("../../resources/completions/ssh.json"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_locale_prefers_exact_match() {
        let mut m = HashMap::new();
        m.insert("en".into(), "build".into());
        m.insert("zh-CN".into(), "构建".into());
        assert_eq!(Library::pick_locale(&m, "zh-CN"), "构建");
        assert_eq!(Library::pick_locale(&m, "en"), "build");
    }

    #[test]
    fn pick_locale_falls_back_to_language_root_then_en() {
        let mut m = HashMap::new();
        m.insert("en".into(), "build".into());
        m.insert("zh".into(), "构建".into());
        // zh-TW not present, but the language root `zh` is.
        assert_eq!(Library::pick_locale(&m, "zh-TW"), "构建");
        // ja not present at all → falls all the way back to en.
        assert_eq!(Library::pick_locale(&m, "ja"), "build");
    }

    #[test]
    fn pick_locale_finds_region_when_only_bare_language_requested() {
        // User asked for "zh" but the pack only has "zh-CN" — we
        // should still find it.
        let mut m = HashMap::new();
        m.insert("en".into(), "build".into());
        m.insert("zh-CN".into(), "构建".into());
        assert_eq!(Library::pick_locale(&m, "zh"), "构建");
    }

    #[test]
    fn pick_locale_empty_string_when_nothing_present() {
        let m: HashMap<String, String> = HashMap::new();
        assert_eq!(Library::pick_locale(&m, "en"), "");
    }

    #[test]
    fn bundled_packs_parse_and_have_consistent_schema() {
        let lib = Library::bundled();
        assert!(
            !lib.is_empty(),
            "bundled() should ship at least one pack"
        );
        for name in lib.commands() {
            let pack = lib.lookup(name).unwrap();
            assert_eq!(
                pack.schema_version, SCHEMA_VERSION,
                "pack {name} has wrong schema_version"
            );
            assert_eq!(pack.command, name, "pack {name} self-name mismatch");
        }
    }

    #[test]
    fn merge_dir_loads_only_json_files_and_skips_malformed() {
        let tmp = std::env::temp_dir().join(format!(
            "pier-x-lib-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        // Valid pack.
        std::fs::write(
            tmp.join("foo.json"),
            r#"{"schema_version":1,"command":"foo","tool_version":"","source":"user","import_method":"hand-curated","import_date":"","subcommands":[],"options":[]}"#,
        ).unwrap();
        // Garbage — should be skipped without crashing.
        std::fs::write(tmp.join("bar.json"), "not json").unwrap();
        // Wrong extension — ignored.
        std::fs::write(tmp.join("baz.txt"), "{}").unwrap();
        // Future schema — refused.
        std::fs::write(
            tmp.join("future.json"),
            r#"{"schema_version":99,"command":"future","tool_version":"","source":"user","import_method":"","import_date":"","subcommands":[],"options":[]}"#,
        ).unwrap();

        let mut lib = Library::empty();
        let loaded = lib.merge_dir(&tmp);
        assert_eq!(loaded, 1);
        assert!(lib.lookup("foo").is_some());
        assert!(lib.lookup("future").is_none());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn merge_dir_returns_zero_when_directory_missing() {
        let lib_count = {
            let mut lib = Library::empty();
            lib.merge_dir(Path::new("/nonexistent/path/that/does/not/exist"))
        };
        assert_eq!(lib_count, 0);
    }

    #[test]
    fn library_inserts_replace_existing_pack_for_same_command() {
        let mut lib = Library::empty();
        let mut a = CommandPack {
            schema_version: SCHEMA_VERSION,
            command: "foo".into(),
            tool_version: "1.0".into(),
            source: "bundled-seed".into(),
            import_method: "hand-curated".into(),
            import_date: String::new(),
            subcommands: Vec::new(),
            options: Vec::new(),
        };
        lib.insert(a.clone());
        a.tool_version = "2.0".into();
        a.source = "user".into();
        lib.insert(a);
        let resolved = lib.lookup("foo").unwrap();
        assert_eq!(resolved.tool_version, "2.0");
        assert_eq!(resolved.source, "user");
    }
}
