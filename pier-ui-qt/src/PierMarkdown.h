// ─────────────────────────────────────────────────────────
// PierMarkdown — QML singleton for local Markdown preview
// ─────────────────────────────────────────────────────────
//
// The simplest service in the app. Three Q_INVOKABLE methods
// that wrap the pier_markdown_* C ABI:
//
//   * toHtml(source)  — render in-memory markdown to HTML
//   * loadHtml(path)  — read+render a local file to HTML
//   * loadSource(path) — read a local file as raw markdown
//
// Why a singleton and not a QObject-per-view?
// ───────────────────────────────────────────
//   Markdown rendering is pure, stateless, and fast —
//   pulldown-cmark does 16 MB in a few milliseconds on an M2.
//   There's nothing to cache, no session to keep alive, and
//   nothing that would benefit from cross-view coordination.
//   A singleton keeps the QML side trivial: `PierMarkdown.toHtml(src)`.
//
// Threading
// ─────────
//   Every method runs on the Qt main thread. The underlying
//   Rust functions are synchronous and non-blocking in the
//   Qt sense — no FFI worker thread is needed. If we ever
//   need to render a pathologically large file without
//   stalling the UI, the right fix is to split the render
//   at the Rust layer, not move this to a QObject.

#pragma once

#include <QObject>
#include <QString>
#include <qqml.h>

class PierMarkdown : public QObject
{
    Q_OBJECT
    QML_NAMED_ELEMENT(PierMarkdown)
    QML_SINGLETON

public:
    explicit PierMarkdown(QObject *parent = nullptr) : QObject(parent) {}

    /// Render `source` (UTF-8 markdown) to HTML. Returns an
    /// empty string on failure — the QML side can show a
    /// placeholder in that case.
    Q_INVOKABLE QString toHtml(const QString &source) const;

    /// Read `path` as UTF-8 markdown and return the rendered
    /// HTML. Returns an empty string on I/O error or
    /// oversized file.
    Q_INVOKABLE QString loadHtml(const QString &path) const;

    /// Read `path` as UTF-8 markdown and return the raw
    /// contents (no rendering). Used by the split view's
    /// source pane.
    Q_INVOKABLE QString loadSource(const QString &path) const;
};
