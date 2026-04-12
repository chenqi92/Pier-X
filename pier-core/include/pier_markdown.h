/*
 * pier-core — Local Markdown preview C ABI
 * ─────────────────────────────────────────
 *
 * M5e per-service tool. The simplest FFI in pier-core: three
 * pure functions + one free. No handles, no sessions, no
 * worker threads — pulldown-cmark is fast enough (16 MB in a
 * few milliseconds) that the C++ side just calls these
 * synchronously from its QML singleton.
 *
 * Security: raw HTML in the markdown source is ALWAYS
 * escaped, not passed through. A hostile README.md that
 * contains `<script>` becomes literal text in the rendered
 * output, not live markup.
 *
 * Memory ownership:
 *   * Strings returned by any `pier_markdown_*` function are
 *     owned by Rust and must be released via
 *     pier_markdown_free_string. Do NOT use C free().
 */

#ifndef PIER_MARKDOWN_H
#define PIER_MARKDOWN_H

#ifdef __cplusplus
extern "C" {
#endif

/* Render UTF-8 markdown to heap UTF-8 HTML. Returns NULL on
 * null / non-UTF-8 input, or if the output contains an
 * interior NUL. Release with pier_markdown_free_string. */
char *pier_markdown_render_html(const char *source);

/* Read a local file as UTF-8 markdown and render it to HTML.
 * Same semantics as pier_markdown_render_html above, but also
 * returns NULL on I/O error, missing file, non-UTF-8 content,
 * or files larger than the internal 16 MB cap. Release with
 * pier_markdown_free_string. */
char *pier_markdown_load_html(const char *path);

/* Read a local file as UTF-8 markdown and return its raw
 * contents (no rendering). Used by the "Source" split pane.
 * Same null / error contract as pier_markdown_load_html.
 * Release with pier_markdown_free_string. */
char *pier_markdown_load_source(const char *path);

/* Release a string returned by any pier_markdown_* function.
 * Safe to call with NULL. */
void pier_markdown_free_string(char *s);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* PIER_MARKDOWN_H */
