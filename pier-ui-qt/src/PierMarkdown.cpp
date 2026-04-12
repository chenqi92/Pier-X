#include "PierMarkdown.h"

#include "pier_markdown.h"

#include <QByteArray>

namespace {

/// Call a pier_markdown_* function that takes a single
/// `const char *` input and returns a heap C string. Wraps
/// the allocate → copy → free dance into a single expression.
template <typename Fn>
QString callRust(const QString &input, Fn fn)
{
    const QByteArray utf8 = input.toUtf8();
    char *raw = fn(utf8.constData());
    if (!raw) {
        return QString();
    }
    QString out = QString::fromUtf8(raw);
    pier_markdown_free_string(raw);
    return out;
}

} // namespace

QString PierMarkdown::toHtml(const QString &source) const
{
    if (source.isEmpty()) return QString();
    return callRust(source, pier_markdown_render_html);
}

QString PierMarkdown::loadHtml(const QString &path) const
{
    if (path.isEmpty()) return QString();
    return callRust(path, pier_markdown_load_html);
}

QString PierMarkdown::loadSource(const QString &path) const
{
    if (path.isEmpty()) return QString();
    return callRust(path, pier_markdown_load_source);
}
