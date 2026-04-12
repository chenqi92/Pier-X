//! Markdown → HTML rendering for the local preview panel (M5e).
//!
//! ## Scope
//!
//! This module is deliberately small. It wraps `pulldown-cmark`
//! with a fixed set of extensions that match GitHub-flavored
//! Markdown at the "typical README.md" level — tables,
//! strikethrough, task lists, footnotes, heading anchors. We
//! do **not** pass raw HTML through: any `<script>` or
//! `<iframe>` in a markdown file is escaped to its source
//! form. The file we're rendering comes from the user's
//! filesystem, but we make no assumptions about who wrote it,
//! and Qt's rich-text engine will happily execute some things
//! we'd rather not.
//!
//! ## Why not use Qt's `Text.MarkdownText`?
//!
//! Qt gained native markdown rendering in 5.14, but its
//! CommonMark-md4c fork has different extension defaults and
//! stricter HTML handling on different platforms. Rendering
//! in Rust gives us one source of truth across every OS we
//! ship on, and the same `render_html` is unit-tested in CI
//! before it ever reaches a Qt widget.
//!
//! ## Not yet
//!
//! * Syntax highlighting inside fenced code blocks. M5e+
//!   would plug in `syntect` or a classless prism-style CSS
//!   class set for the Qt side to style.
//! * Live reload on file change. The UI owns a manual
//!   "Reload" button today; a `notify`-backed watcher lives
//!   with M6 polish if it lands.
//! * Mermaid / math. Neither render in Qt's rich-text anyway,
//!   so not a regression.

use std::fs;
use std::io;
use std::path::Path;

use pulldown_cmark::{html, CowStr, Event, Options, Parser};

/// Max file size we'll load for preview. 16 MB is well above
/// any sane README and protects the UI from pasting a DB
/// dump into the viewer by accident.
pub const MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;

/// Extensions we enable on every render. Kept as a single
/// function so tests and the FFI both agree on the exact
/// flag set — regressions in GFM compat become trivially
/// diffable.
fn gfm_options() -> Options {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_FOOTNOTES);
    opts.insert(Options::ENABLE_HEADING_ATTRIBUTES);
    opts.insert(Options::ENABLE_SMART_PUNCTUATION);
    // Deliberately NOT enabled:
    //   * ENABLE_OLD_FOOTNOTES — deprecated in pulldown-cmark.
    //   * (raw HTML passthrough is disabled by default — good.)
    opts
}

/// Render CommonMark + GFM-ish markdown to HTML. Never panics,
/// always returns a string (empty input → empty output).
///
/// Raw HTML embedded in the source is **escaped**, not passed
/// through: `<script>...</script>` in a .md file becomes
/// literal `&lt;script&gt;...&lt;/script&gt;` in the output.
/// pulldown-cmark's default passthrough would let a hostile
/// markdown file inject arbitrary markup into the preview,
/// which Qt's rich-text engine would happily render.
pub fn render_html(source: &str) -> String {
    let parser = Parser::new_ext(source, gfm_options()).map(sanitize_html_events);
    let mut out = String::with_capacity(source.len() + source.len() / 4);
    html::push_html(&mut out, parser);
    out
}

/// Event filter: replace [`Event::Html`] / [`Event::InlineHtml`]
/// with [`Event::Text`] containing the same bytes. That makes
/// pulldown-cmark's HTML renderer emit them through its text
/// escaper, turning angle brackets into entities.
fn sanitize_html_events(event: Event<'_>) -> Event<'_> {
    match event {
        Event::Html(s) | Event::InlineHtml(s) => Event::Text(CowStr::from(s.into_string())),
        other => other,
    }
}

/// Load a file from disk as UTF-8, returning `Err` on missing,
/// too-large, or non-UTF-8 files. Used by the preview FFI so
/// the UI never has to touch std::fs directly.
///
/// Files larger than [`MAX_SOURCE_BYTES`] are rejected before
/// being read into memory.
pub fn load_file(path: &Path) -> io::Result<String> {
    let metadata = fs::metadata(path)?;
    if metadata.len() > MAX_SOURCE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "file too large ({} bytes, max {})",
                metadata.len(),
                MAX_SOURCE_BYTES
            ),
        ));
    }
    fs::read_to_string(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_headings() {
        let html = render_html("# Hello\n\n## World");
        assert!(html.contains("<h1>Hello</h1>"));
        assert!(html.contains("<h2>World</h2>"));
    }

    #[test]
    fn renders_unordered_list() {
        let html = render_html("- one\n- two\n- three");
        assert!(html.contains("<ul>"));
        assert!(html.contains("<li>one</li>"));
        assert!(html.contains("<li>two</li>"));
        assert!(html.contains("<li>three</li>"));
    }

    #[test]
    fn renders_ordered_list() {
        let html = render_html("1. first\n2. second");
        assert!(html.contains("<ol>"));
        assert!(html.contains("<li>first</li>"));
        assert!(html.contains("<li>second</li>"));
    }

    #[test]
    fn renders_fenced_code_block() {
        let source = "```rust\nfn main() {}\n```";
        let html = render_html(source);
        assert!(html.contains("<pre>"));
        assert!(html.contains("<code"));
        assert!(html.contains("fn main() {}"));
    }

    #[test]
    fn renders_inline_code() {
        let html = render_html("use `Foo::bar()` here");
        assert!(html.contains("<code>Foo::bar()</code>"));
    }

    #[test]
    fn renders_emphasis_and_strong() {
        let html = render_html("*em* and **strong**");
        assert!(html.contains("<em>em</em>"));
        assert!(html.contains("<strong>strong</strong>"));
    }

    #[test]
    fn renders_gfm_strikethrough() {
        let html = render_html("~~gone~~");
        assert!(html.contains("<del>gone</del>"));
    }

    #[test]
    fn renders_gfm_table() {
        let md = "| h1 | h2 |\n| --- | --- |\n| a | b |";
        let html = render_html(md);
        assert!(html.contains("<table>"));
        assert!(html.contains("<th>h1</th>"));
        assert!(html.contains("<td>a</td>"));
    }

    #[test]
    fn renders_gfm_task_list() {
        let md = "- [x] done\n- [ ] todo";
        let html = render_html(md);
        // pulldown-cmark emits <input disabled="" type="checkbox"/>
        assert!(html.contains("type=\"checkbox\""));
        assert!(html.contains("checked"));
    }

    #[test]
    fn renders_links() {
        let html = render_html("[pier](https://example.com)");
        assert!(html.contains("<a href=\"https://example.com\""));
        assert!(html.contains(">pier</a>"));
    }

    #[test]
    fn escapes_html_in_source_by_default() {
        // Raw HTML in the source should be treated as literal
        // text (escaped), not as live markup — this is the
        // safety rationale for not enabling pulldown-cmark's
        // HTML passthrough option.
        let html = render_html("look: <script>alert(1)</script>");
        assert!(!html.contains("<script>alert(1)</script>"));
        // Still wraps in a paragraph though.
        assert!(html.contains("<p>"));
    }

    #[test]
    fn empty_input_is_empty_output() {
        assert_eq!(render_html(""), "");
    }

    #[test]
    fn load_file_reads_small_markdown_from_temp() {
        use std::io::Write;
        // Use a per-pid suffix so concurrent test binaries
        // don't step on each other. tempfile isn't a dev-dep
        // for pier-core, and load_file is trivial enough that
        // a hand-rolled temp path suffices.
        let path = std::env::temp_dir().join(format!("pier_md_test_{}.md", std::process::id()));
        {
            let mut f = std::fs::File::create(&path).expect("create temp md");
            writeln!(f, "# hi").unwrap();
        }
        let contents = load_file(&path).expect("load");
        assert!(contents.starts_with("# hi"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn max_source_bytes_is_reasonable() {
        // Sanity: 16 MB is enough for any realistic README
        // (kernel docs cap around 600 KB), but small enough
        // that Qt's rich-text engine can still render the
        // result without stalling the UI thread.
        const _: () = assert!(MAX_SOURCE_BYTES >= 1024 * 1024);
        const _: () = assert!(MAX_SOURCE_BYTES <= 64 * 1024 * 1024);
    }

    #[test]
    fn load_file_rejects_missing_path() {
        let result = load_file(Path::new("/no/such/file/exists.md"));
        assert!(result.is_err());
    }
}
