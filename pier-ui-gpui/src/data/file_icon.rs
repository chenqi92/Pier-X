//! Extension → icon/color mapping for the file-browser row.
//!
//! Mirrors `Pier/PierApp/Sources/Models/FileItem.swift`'s `iconName` /
//! `iconColor`, translated from SF Symbols to Lucide-style glyphs that
//! pier-ui-gpui actually ships. The hint shape stays the same so the
//! UI reads identically across the two apps:
//!   - code files read as "code"
//!   - markdown reads as "docs"
//!   - images read as "palette"
//!   - archives read as "package"
//!   - configs read as "settings"
//!   - shell scripts read as "terminal"
//!   - web files read as "globe"
//!
//! Icons are chosen from variants where a matching SVG is registered in
//! `crate::assets` — see that module if you add new extensions and the
//! glyph shows up blank.
//!
//! `FileIconTone` is intentionally semantic (not raw `Rgba`) so the
//! renderer can pick the actual color off the active theme — this
//! keeps dark / light parity and avoids literal colors in views per
//! CLAUDE.md Rule 1.

use gpui_component::IconName;

/// Semantic color bucket for a file-type glyph. Maps to a concrete
/// `theme.color.*` in the file-tree row renderer.
#[derive(Clone, Copy, Debug)]
pub enum FileIconTone {
    /// Folders — accent blue, the primary "navigable target" cue.
    Directory,
    /// Markdown, TeX, rich docs — pairs with the book-open glyph.
    Docs,
    /// Source code in any language.
    Code,
    /// Config formats: JSON / TOML / YAML / INI / XML.
    Config,
    /// Shell scripts and batch files.
    Shell,
    /// HTML / CSS / web assets.
    Web,
    /// Images (png / jpg / svg / …).
    Image,
    /// Video / audio media.
    Media,
    /// Archives (zip / tar / …).
    Archive,
    /// Everything else — falls back to the theme's tertiary text color.
    Neutral,
}

/// Pick an `(icon, tone)` pair for a filesystem entry.
///
/// - `is_dir = true` short-circuits to `(Folder, Directory)` regardless
///   of the name — matches Pier's behavior.
/// - For files, the extension is matched case-insensitively; a few
///   dotfiles (`.gitignore`, `.gitattributes`, `Dockerfile`, `Makefile`)
///   are recognized by full name as a nicety.
pub fn file_icon(name: &str, is_dir: bool) -> (IconName, FileIconTone) {
    if is_dir {
        return (IconName::Folder, FileIconTone::Directory);
    }

    // Whole-name matches first — these don't have an extension but
    // have an obvious "role" glyph.
    match name {
        ".gitignore" | ".gitattributes" | ".gitmodules" => {
            return (IconName::GitBranch, FileIconTone::Code);
        }
        "Dockerfile" | "docker-compose.yml" | "docker-compose.yaml" => {
            return (IconName::Container, FileIconTone::Config);
        }
        "Makefile" | "makefile" | "CMakeLists.txt" => {
            return (IconName::Settings, FileIconTone::Config);
        }
        "LICENSE" | "LICENSE.md" | "LICENSE.txt" | "README" => {
            return (IconName::BookOpen, FileIconTone::Docs);
        }
        _ => {}
    }

    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();

    match ext.as_str() {
        // Source code.
        "rs" | "swift" | "py" | "js" | "ts" | "jsx" | "tsx" | "mjs" | "cjs" | "go" | "java"
        | "kt" | "kts" | "scala" | "rb" | "php" | "lua" | "dart" | "cs" | "fs" | "hs" | "ex"
        | "exs" | "erl" | "clj" | "c" | "cc" | "cpp" | "cxx" | "h" | "hpp" => {
            (IconName::SquareTerminal, FileIconTone::Code)
        }

        // Docs / prose.
        "md" | "markdown" | "mdx" | "rst" | "adoc" | "tex" => {
            (IconName::BookOpen, FileIconTone::Docs)
        }

        // Plain text.
        "txt" | "log" => (IconName::FileText, FileIconTone::Neutral),

        // Structured configs.
        "json" | "toml" | "yaml" | "yml" | "xml" | "ini" | "conf" | "env" | "properties" => {
            (IconName::Settings, FileIconTone::Config)
        }

        // Images.
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "bmp" | "ico" | "tif" | "tiff" => {
            (IconName::Palette, FileIconTone::Image)
        }

        // Video / audio.
        "mp4" | "mov" | "avi" | "mkv" | "webm" | "m4v" | "mp3" | "wav" | "flac" | "ogg"
        | "aac" | "m4a" => (IconName::Play, FileIconTone::Media),

        // Archives — Container glyph reads as "package".
        "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "7z" | "rar" | "dmg" | "iso" => {
            (IconName::Container, FileIconTone::Archive)
        }

        // Shell scripts.
        "sh" | "zsh" | "bash" | "fish" | "ps1" | "bat" | "cmd" => {
            (IconName::SquareTerminal, FileIconTone::Shell)
        }

        // Web.
        "html" | "htm" | "css" | "scss" | "sass" | "less" => (IconName::Globe, FileIconTone::Web),

        // Anything else — generic file glyph.
        _ => (IconName::File, FileIconTone::Neutral),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_wins_over_extension() {
        let (icon, tone) = file_icon("src.rs", true);
        assert!(matches!(icon, IconName::Folder));
        assert!(matches!(tone, FileIconTone::Directory));
    }

    #[test]
    fn rust_source_uses_code_terminal() {
        let (icon, tone) = file_icon("main.rs", false);
        assert!(matches!(icon, IconName::SquareTerminal));
        assert!(matches!(tone, FileIconTone::Code));
    }

    #[test]
    fn markdown_uses_book_open() {
        let (icon, tone) = file_icon("README.md", false);
        assert!(matches!(icon, IconName::BookOpen));
        assert!(matches!(tone, FileIconTone::Docs));
    }

    #[test]
    fn gitignore_matched_by_name() {
        let (icon, tone) = file_icon(".gitignore", false);
        assert!(matches!(icon, IconName::GitBranch));
        assert!(matches!(tone, FileIconTone::Code));
    }

    #[test]
    fn unknown_extension_falls_back_to_file() {
        let (icon, tone) = file_icon("blob.xyz", false);
        assert!(matches!(icon, IconName::File));
        assert!(matches!(tone, FileIconTone::Neutral));
    }
}
