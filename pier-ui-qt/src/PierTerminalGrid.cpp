#include "PierTerminalGrid.h"
#include "PierTerminalSession.h"

#include "pier_terminal.h"

#include <QFontMetricsF>
#include <QPainter>
#include <QRegularExpression>
#include <QStringList>
#include <QVector>

#include <cmath>

namespace {

// Minimal 16-color ANSI palette. When a custom palette is supplied
// (from QML via PierTerminalGrid::paletteColors), use it directly.
// Otherwise fall back to the built-in dark/light palettes.
QColor ansiIndexToColor(uint8_t idx, const QColor &defaultFg, bool isDark,
                        const QList<QColor> &palette)
{
    if (idx < 16) {
        if (palette.size() >= 16)
            return palette[idx];
        if (isDark) {
            static const QColor darkPalette[16] = {
                QColor(0x00, 0x00, 0x00), QColor(0xCD, 0x00, 0x00),
                QColor(0x00, 0xCD, 0x00), QColor(0xCD, 0xCD, 0x00),
                QColor(0x3B, 0x78, 0xFF), QColor(0xCD, 0x00, 0xCD),
                QColor(0x00, 0xCD, 0xCD), QColor(0xE5, 0xE5, 0xE5),
                QColor(0x7F, 0x7F, 0x7F), QColor(0xFF, 0x00, 0x00),
                QColor(0x00, 0xFF, 0x00), QColor(0xFF, 0xFF, 0x00),
                QColor(0x5C, 0x5C, 0xFF), QColor(0xFF, 0x00, 0xFF),
                QColor(0x00, 0xFF, 0xFF), QColor(0xFF, 0xFF, 0xFF),
            };
            return darkPalette[idx];
        }

        static const QColor lightPalette[16] = {
            QColor(0x00, 0x00, 0x00), QColor(0xCD, 0x00, 0x00),
            QColor(0x00, 0xA0, 0x00), QColor(0xA0, 0x70, 0x00),
            QColor(0x00, 0x00, 0xEE), QColor(0xCD, 0x00, 0xCD),
            QColor(0x00, 0xA0, 0xA0), QColor(0x66, 0x66, 0x66),
            QColor(0x55, 0x55, 0x55), QColor(0xFF, 0x00, 0x00),
            QColor(0x00, 0xCD, 0x00), QColor(0xCD, 0xCD, 0x00),
            QColor(0x5C, 0x5C, 0xFF), QColor(0xFF, 0x00, 0xFF),
            QColor(0x00, 0xCD, 0xCD), QColor(0x44, 0x44, 0x44),
        };
        return lightPalette[idx];
    }

    if (idx >= 16 && idx <= 231) {
        const int n = idx - 16;
        const int r = (n / 36) % 6;
        const int g = (n / 6) % 6;
        const int b = n % 6;
        const auto toByte = [](int c) { return c == 0 ? 0 : 55 + c * 40; };
        return QColor(toByte(r), toByte(g), toByte(b));
    }
    if (idx >= 232 && idx <= 255) {
        const int gray = 8 + (idx - 232) * 10;
        return QColor(gray, gray, gray);
    }
    return defaultFg;
}

QColor cellForeground(const PierCell &c, const QColor &defaultFg, bool isDark,
                      const QList<QColor> &palette)
{
    switch (c.fg_kind) {
    case 0:  return defaultFg;
    case 1:  return ansiIndexToColor(c.fg_r, defaultFg, isDark, palette);
    case 2:  return QColor(c.fg_r, c.fg_g, c.fg_b);
    default: return defaultFg;
    }
}

QColor cellBackground(const PierCell &c, const QColor &defaultBg, bool isDark,
                      const QList<QColor> &palette)
{
    switch (c.bg_kind) {
    case 0:  return defaultBg;
    case 1:  return ansiIndexToColor(c.bg_r, defaultBg, isDark, palette);
    case 2:  return QColor(c.bg_r, c.bg_g, c.bg_b);
    default: return defaultBg;
    }
}

bool isWideCodepoint(uint32_t cp)
{
    return (cp >= 0x1100 && cp <= 0x115F)
        || cp == 0x2329 || cp == 0x232A
        || (cp >= 0x2E80 && cp <= 0x303E)
        || (cp >= 0x3040 && cp <= 0x33BF)
        || (cp >= 0x3400 && cp <= 0x4DBF)
        || (cp >= 0x4E00 && cp <= 0x9FFF)
        || (cp >= 0xA000 && cp <= 0xA4CF)
        || (cp >= 0xAC00 && cp <= 0xD7AF)
        || (cp >= 0xF900 && cp <= 0xFAFF)
        || (cp >= 0xFE10 && cp <= 0xFE6F)
        || (cp >= 0xFF01 && cp <= 0xFF60)
        || (cp >= 0xFFE0 && cp <= 0xFFE6)
        || (cp >= 0x20000 && cp <= 0x2FFFF)
        || (cp >= 0x30000 && cp <= 0x3FFFF);
}

QString trimTrailingUrlPunctuation(QString value)
{
    static const QString trailing = QStringLiteral(".,;:!?");
    while (!value.isEmpty() && trailing.contains(value.back())) {
        value.chop(1);
    }
    return value;
}

bool isWordSelectionChar(QChar ch)
{
    if (ch.isLetterOrNumber())
        return true;
    static const QString extras = QStringLiteral("_-./\\:@%+=~#$");
    return extras.contains(ch);
}

QString cellString(uint32_t codepoint)
{
    if (codepoint == 0)
        return QString();
    const char32_t cp = static_cast<char32_t>(codepoint);
    return QString::fromUcs4(&cp, 1);
}

} // namespace

PierTerminalGrid::PierTerminalGrid(QQuickItem *parent)
    : QQuickPaintedItem(parent)
{
    m_font = QFont(QStringLiteral("JetBrains Mono"));
    m_font.setPointSize(13);
    m_font.setStyleHint(QFont::Monospace);
    recomputeMetrics();

    setAntialiasing(true);
    setFlag(ItemHasContents, true);

    m_blinkTimer.setInterval(530);
    connect(&m_blinkTimer, &QTimer::timeout, this, [this]() {
        m_cursorBlinkVisible = !m_cursorBlinkVisible;
        update();
    });
    if (m_cursorBlink)
        m_blinkTimer.start();
}

void PierTerminalGrid::setSession(PierTerminalSession *s)
{
    if (m_session == s) {
        return;
    }
    if (m_session) {
        disconnect(m_session, nullptr, this, nullptr);
    }
    m_session = s;
    clearSelection();
    clearHoveredUrl();
    if (m_session) {
        connect(m_session, &PierTerminalSession::gridChanged,
                this, &PierTerminalGrid::onSessionGridChanged);
    }
    emit sessionChanged();
    update();
}

void PierTerminalGrid::setFont(const QFont &f)
{
    if (m_font == f) {
        return;
    }
    m_font = f;
    recomputeMetrics();
    fitToViewport();
    emit fontChanged();
    emit metricsChanged();
    update();
}

void PierTerminalGrid::setDefaultForeground(const QColor &c)
{
    if (m_defaultFg == c) return;
    m_defaultFg = c;
    emit defaultForegroundChanged();
    update();
}

void PierTerminalGrid::setDefaultBackground(const QColor &c)
{
    if (m_defaultBg == c) return;
    m_defaultBg = c;
    emit defaultBackgroundChanged();
    update();
}

void PierTerminalGrid::setIsDarkTheme(bool dark)
{
    if (m_isDarkTheme == dark) return;
    m_isDarkTheme = dark;
    emit isDarkThemeChanged();
    update();
}

void PierTerminalGrid::setPaletteColors(const QList<QColor> &colors)
{
    if (m_palette == colors) return;
    m_palette = colors;
    emit paletteColorsChanged();
    update();
}

void PierTerminalGrid::setSelectionBackground(const QColor &color)
{
    if (m_selectionBg == color)
        return;
    m_selectionBg = color;
    emit selectionAppearanceChanged();
    update();
}

void PierTerminalGrid::setLinkForeground(const QColor &color)
{
    if (m_linkFg == color)
        return;
    m_linkFg = color;
    emit linkColorsChanged();
    update();
}

void PierTerminalGrid::setLinkHoverForeground(const QColor &color)
{
    if (m_linkHoverFg == color)
        return;
    m_linkHoverFg = color;
    emit linkColorsChanged();
    update();
}

bool PierTerminalGrid::hasSelection() const
{
    return m_selectionAnchor.x() >= 0
        && m_selectionAnchor.y() >= 0
        && m_selectionExtent.x() >= 0
        && m_selectionExtent.y() >= 0;
}

void PierTerminalGrid::setCursorStyle(int style)
{
    style = qBound(0, style, 2);
    if (m_cursorStyle == style) return;
    m_cursorStyle = style;
    emit cursorStyleChanged();
    update();
}

void PierTerminalGrid::setCursorBlink(bool blink)
{
    if (m_cursorBlink == blink) return;
    m_cursorBlink = blink;
    if (blink) {
        m_cursorBlinkVisible = true;
        m_blinkTimer.start();
    } else {
        m_blinkTimer.stop();
        m_cursorBlinkVisible = true;
        update();
    }
    emit cursorBlinkChanged();
}

void PierTerminalGrid::setCursorVisible(bool visible)
{
    if (m_cursorVisible == visible) return;
    m_cursorVisible = visible;
    emit cursorVisibleChanged();
    update();
}

void PierTerminalGrid::geometryChange(const QRectF &newGeometry, const QRectF &oldGeometry)
{
    QQuickPaintedItem::geometryChange(newGeometry, oldGeometry);
    if (newGeometry.size() != oldGeometry.size()) {
        fitToViewport();
    }
}

void PierTerminalGrid::onSessionGridChanged()
{
    update();
}

void PierTerminalGrid::fitToViewport()
{
    if (!m_session || m_cellWidth <= 0 || m_cellHeight <= 0) {
        return;
    }
    const int cols = qMax(1, int(width() / m_cellWidth));
    const int rows = qMax(1, int(height() / m_cellHeight));
    if (cols != m_session->cols() || rows != m_session->rows()) {
        clearSelection();
        clearHoveredUrl();
        m_session->resize(cols, rows);
        emit metricsChanged();
    }
}

void PierTerminalGrid::recomputeMetrics()
{
    const QFontMetricsF fm(m_font);
    const qreal mWidth = fm.horizontalAdvance(QChar('M'));
    m_cellWidth = mWidth > 0 ? mWidth : fm.averageCharWidth();

    // Use lineSpacing() instead of height() — lineSpacing includes the
    // inter-line leading which prevents row overlap. On Windows with CJK
    // fallback fonts (e.g. Microsoft YaHei), the fallback font's metrics
    // can exceed the primary monospace font's height, causing rows to
    // overlap if we only use height(). We also measure a representative
    // CJK character and take the maximum to be safe.
    qreal baseHeight = qMax(fm.height(), fm.lineSpacing());

#ifdef Q_OS_WIN
    // CJK fallback fonts on Windows often have larger bounding boxes.
    // Measure a representative character so the cell height accommodates them.
    const QRectF cjkBounds = fm.boundingRect(QChar(0x4E2D)); // U+4E2D '中'
    if (cjkBounds.height() > baseHeight)
        baseHeight = cjkBounds.height();
#endif

    // Ceil to avoid fractional-pixel accumulation that shifts rows by
    // sub-pixel amounts over many lines, eventually causing visible overlap.
    m_cellHeight = std::ceil(baseHeight);
    m_ascent = fm.ascent();
}

QPoint PierTerminalGrid::cellAt(qreal x, qreal y, int cols, int rows, bool clampToBounds) const
{
    if (cols <= 0 || rows <= 0 || m_cellWidth <= 0 || m_cellHeight <= 0) {
        return QPoint(-1, -1);
    }

    int col = static_cast<int>(x / m_cellWidth);
    int row = static_cast<int>(y / m_cellHeight);
    if (clampToBounds) {
        col = qBound(0, col, cols - 1);
        row = qBound(0, row, rows - 1);
        return QPoint(col, row);
    }
    if (col < 0 || col >= cols || row < 0 || row >= rows) {
        return QPoint(-1, -1);
    }
    return QPoint(col, row);
}

QString PierTerminalGrid::lineText(const PierCell *cells, int row, int cols) const
{
    QString text;
    text.reserve(cols);
    for (int col = 0; col < cols; ++col) {
        const uint32_t ch = cells[row * cols + col].ch;
        if (ch == 0) {
            text.append(QLatin1Char(' '));
        } else if (ch <= 0xFFFF) {
            text.append(QChar(static_cast<char16_t>(ch)));
        } else {
            text.append(QChar(u'?'));
        }
    }
    return text;
}

bool PierTerminalGrid::isWideLeadingCell(const PierCell *cells, int row, int col, int cols) const
{
    if (!cells || col < 0 || col + 1 >= cols || row < 0) {
        return false;
    }
    const PierCell &current = cells[row * cols + col];
    const PierCell &next = cells[row * cols + col + 1];
    return current.ch != 0 && next.ch == 0 && isWideCodepoint(current.ch);
}

PierTerminalGrid::UrlMatch PierTerminalGrid::urlMatchAt(int row, int col,
                                                        const PierCell *cells,
                                                        int cols, int rows) const
{
    if (!cells || row < 0 || row >= rows || col < 0 || col >= cols) {
        return {};
    }

    static const QRegularExpression urlRegex(
        QStringLiteral(R"(https?://[^\s"'<>\)\]]+)"));

    const QString rowText = lineText(cells, row, cols);
    QRegularExpressionMatchIterator it = urlRegex.globalMatch(rowText);
    while (it.hasNext()) {
        const QRegularExpressionMatch match = it.next();
        QString url = trimTrailingUrlPunctuation(match.captured(0));
        if (url.isEmpty())
            continue;
        const int startCol = match.capturedStart();
        const int endCol = startCol + url.size();
        if (col >= startCol && col < endCol) {
            return {row, startCol, endCol, url};
        }
    }

    return {};
}

PierTerminalGrid::UrlMatch PierTerminalGrid::hoveredMatchAt(qreal x, qreal y) const
{
    if (!m_session || m_cellWidth <= 0 || m_cellHeight <= 0) {
        return {};
    }

    int cols = 0;
    int rows = 0;
    const PierCell *cells = m_session->rawCells(&cols, &rows);
    if (!cells || cols <= 0 || rows <= 0) {
        return {};
    }

    const QPoint pos = cellAt(x, y, cols, rows, false);
    if (pos.x() < 0 || pos.y() < 0) {
        return {};
    }
    return urlMatchAt(pos.y(), pos.x(), cells, cols, rows);
}

void PierTerminalGrid::setHoveredUrlMatch(const UrlMatch &match)
{
    const bool sameMatch = m_hoveredMatch.row == match.row
        && m_hoveredMatch.startCol == match.startCol
        && m_hoveredMatch.endCol == match.endCol
        && m_hoveredMatch.url == match.url;
    if (sameMatch) {
        return;
    }

    m_hoveredMatch = match;
    m_hoveredUrl = match.url;
    emit hoveredUrlChanged();
    update();
}

void PierTerminalGrid::updateHoveredLink(qreal x, qreal y)
{
    setHoveredUrlMatch(hoveredMatchAt(x, y));
}

void PierTerminalGrid::clearHoveredUrl()
{
    setHoveredUrlMatch({});
}

QString PierTerminalGrid::urlAt(qreal x, qreal y) const
{
    return hoveredMatchAt(x, y).url;
}

QPoint PierTerminalGrid::normalizedSelectionStart() const
{
    if (!hasSelection())
        return QPoint(-1, -1);
    if (m_selectionAnchor.y() < m_selectionExtent.y())
        return m_selectionAnchor;
    if (m_selectionAnchor.y() > m_selectionExtent.y())
        return m_selectionExtent;
    return m_selectionAnchor.x() <= m_selectionExtent.x() ? m_selectionAnchor : m_selectionExtent;
}

QPoint PierTerminalGrid::normalizedSelectionEnd() const
{
    if (!hasSelection())
        return QPoint(-1, -1);
    if (m_selectionAnchor.y() > m_selectionExtent.y())
        return m_selectionAnchor;
    if (m_selectionAnchor.y() < m_selectionExtent.y())
        return m_selectionExtent;
    return m_selectionAnchor.x() >= m_selectionExtent.x() ? m_selectionAnchor : m_selectionExtent;
}

bool PierTerminalGrid::isCellSelected(int row, int col) const
{
    if (!hasSelection())
        return false;

    const QPoint start = normalizedSelectionStart();
    const QPoint end = normalizedSelectionEnd();
    if (row < start.y() || row > end.y()) {
        return false;
    }
    if (start.y() == end.y()) {
        return row == start.y() && col >= start.x() && col <= end.x();
    }
    if (row == start.y())
        return col >= start.x();
    if (row == end.y())
        return col <= end.x();
    return true;
}

void PierTerminalGrid::beginSelection(qreal x, qreal y)
{
    if (!m_session)
        return;

    int cols = 0;
    int rows = 0;
    const PierCell *cells = m_session->rawCells(&cols, &rows);
    if (!cells || cols <= 0 || rows <= 0)
        return;

    const QPoint pos = cellAt(x, y, cols, rows, true);
    if (pos.x() < 0 || pos.y() < 0)
        return;

    const bool changed = m_selectionAnchor != pos || m_selectionExtent != pos || !m_selectionActive;
    m_selectionAnchor = pos;
    m_selectionExtent = pos;
    m_selectionActive = true;
    if (changed) {
        emit selectionChanged();
        update();
    }
}

void PierTerminalGrid::updateSelection(qreal x, qreal y)
{
    if (!m_selectionActive || !m_session)
        return;

    int cols = 0;
    int rows = 0;
    const PierCell *cells = m_session->rawCells(&cols, &rows);
    if (!cells || cols <= 0 || rows <= 0)
        return;

    const QPoint pos = cellAt(x, y, cols, rows, true);
    if (pos.x() < 0 || pos.y() < 0 || pos == m_selectionExtent)
        return;

    m_selectionExtent = pos;
    emit selectionChanged();
    update();
}

void PierTerminalGrid::endSelection()
{
    m_selectionActive = false;
}

void PierTerminalGrid::clearSelection()
{
    if (!hasSelection() && !m_selectionActive)
        return;

    m_selectionActive = false;
    m_selectionAnchor = QPoint(-1, -1);
    m_selectionExtent = QPoint(-1, -1);
    emit selectionChanged();
    update();
}

bool PierTerminalGrid::selectWordAt(qreal x, qreal y)
{
    if (!m_session)
        return false;

    int cols = 0;
    int rows = 0;
    const PierCell *cells = m_session->rawCells(&cols, &rows);
    if (!cells || cols <= 0 || rows <= 0)
        return false;

    const QPoint pos = cellAt(x, y, cols, rows, false);
    if (pos.x() < 0 || pos.y() < 0)
        return false;

    const UrlMatch url = urlMatchAt(pos.y(), pos.x(), cells, cols, rows);
    if (url.isValid()) {
        m_selectionAnchor = QPoint(url.startCol, url.row);
        m_selectionExtent = QPoint(url.endCol - 1, url.row);
        m_selectionActive = false;
        emit selectionChanged();
        update();
        return true;
    }

    const QString rowText = lineText(cells, pos.y(), cols);
    if (pos.x() >= rowText.size())
        return false;

    const QChar clicked = rowText.at(pos.x());
    if (clicked.isSpace()) {
        clearSelection();
        return false;
    }

    int start = pos.x();
    int end = pos.x();
    if (isWordSelectionChar(clicked)) {
        while (start > 0 && isWordSelectionChar(rowText.at(start - 1)))
            --start;
        while (end + 1 < rowText.size() && isWordSelectionChar(rowText.at(end + 1)))
            ++end;
    }

    m_selectionAnchor = QPoint(start, pos.y());
    m_selectionExtent = QPoint(end, pos.y());
    m_selectionActive = false;
    emit selectionChanged();
    update();
    return true;
}

void PierTerminalGrid::selectAll()
{
    if (!m_session)
        return;

    int cols = 0;
    int rows = 0;
    const PierCell *cells = m_session->rawCells(&cols, &rows);
    if (!cells || cols <= 0 || rows <= 0)
        return;

    m_selectionAnchor = QPoint(0, 0);
    m_selectionExtent = QPoint(cols - 1, rows - 1);
    m_selectionActive = false;
    emit selectionChanged();
    update();
}

QString PierTerminalGrid::selectedRowText(const PierCell *cells, int row, int cols,
                                          int startCol, int endCol) const
{
    QString text;
    text.reserve(qMax(0, endCol - startCol + 1));

    for (int col = startCol; col <= endCol; ++col) {
        const PierCell &cell = cells[row * cols + col];
        if (cell.ch == 0) {
            if (col > 0 && isWideLeadingCell(cells, row, col - 1, cols))
                continue;
            text.append(QLatin1Char(' '));
            continue;
        }
        text.append(cellString(cell.ch));
    }

    while (!text.isEmpty() && text.back() == QLatin1Char(' ')) {
        text.chop(1);
    }
    return text;
}

QString PierTerminalGrid::selectedText() const
{
    if (!hasSelection() || !m_session)
        return {};

    int cols = 0;
    int rows = 0;
    const PierCell *cells = m_session->rawCells(&cols, &rows);
    if (!cells || cols <= 0 || rows <= 0)
        return {};

    const QPoint start = normalizedSelectionStart();
    const QPoint end = normalizedSelectionEnd();

    QStringList lines;
    for (int row = start.y(); row <= end.y(); ++row) {
        const int firstCol = row == start.y() ? start.x() : 0;
        const int lastCol = row == end.y() ? end.x() : cols - 1;
        lines.append(selectedRowText(cells, row, cols, firstCol, lastCol));
    }

    while (!lines.isEmpty() && lines.back().isEmpty()) {
        lines.removeLast();
    }
    return lines.join(QLatin1Char('\n'));
}

void PierTerminalGrid::paint(QPainter *painter)
{
    painter->fillRect(boundingRect(), m_defaultBg);

    if (!m_session) {
        return;
    }

    int cols = 0;
    int rows = 0;
    const PierCell *cells = m_session->rawCells(&cols, &rows);
    if (!cells || cols <= 0 || rows <= 0) {
        return;
    }

    painter->setFont(m_font);

    for (int row = 0; row < rows; ++row) {
        for (int col = 0; col < cols; ++col) {
            const PierCell &cell = cells[row * cols + col];
            if (cell.bg_kind == 0)
                continue;
            const QColor bg = cellBackground(cell, m_defaultBg, m_isDarkTheme, m_palette);
            painter->fillRect(QRectF(col * m_cellWidth, row * m_cellHeight, m_cellWidth, m_cellHeight), bg);
        }
    }

    if (hasSelection()) {
        for (int row = 0; row < rows; ++row) {
            for (int col = 0; col < cols; ++col) {
                if (!isCellSelected(row, col))
                    continue;
                painter->fillRect(QRectF(col * m_cellWidth, row * m_cellHeight, m_cellWidth, m_cellHeight),
                                  m_selectionBg);
            }
        }
    }

    static const QRegularExpression urlRegex(
        QStringLiteral(R"(https?://[^\s"'<>\)\]]+)"));

    for (int row = 0; row < rows; ++row) {
        QVector<UrlMatch> urlMatches;
        const QString rowText = lineText(cells, row, cols);
        QRegularExpressionMatchIterator matchIt = urlRegex.globalMatch(rowText);
        while (matchIt.hasNext()) {
            const QRegularExpressionMatch match = matchIt.next();
            const QString url = trimTrailingUrlPunctuation(match.captured(0));
            if (url.isEmpty())
                continue;
            const int startCol = match.capturedStart();
            const int endCol = startCol + static_cast<int>(url.size());
            urlMatches.push_back({row, startCol, endCol, url});
        }

        for (int col = 0; col < cols; ++col) {
            const PierCell &cell = cells[row * cols + col];
            if (cell.ch == 0 || cell.ch == static_cast<uint32_t>(' '))
                continue;

            bool isWide = isWideLeadingCell(cells, row, col, cols);
            const qreal drawWidth = isWide ? m_cellWidth * 2.0 : m_cellWidth;
            const qreal x = col * m_cellWidth;
            const qreal y = row * m_cellHeight + m_ascent;
            const bool selected = isCellSelected(row, col);

            bool isUrlCell = false;
            bool isHoveredUrlCell = false;
            for (const UrlMatch &match : urlMatches) {
                if (match.contains(row, col)) {
                    isUrlCell = true;
                    isHoveredUrlCell = m_hoveredMatch.isValid() && m_hoveredMatch.contains(row, col);
                    break;
                }
            }

            QColor fg = cellForeground(cell, m_defaultFg, m_isDarkTheme, m_palette);
            if (isUrlCell) {
                fg = isHoveredUrlCell ? m_linkHoverFg : m_linkFg;
            }

            if ((cell.attrs & 0x04) && !selected) {
                const QColor bg = cellBackground(cell, m_defaultBg, m_isDarkTheme, m_palette);
                painter->fillRect(QRectF(x, row * m_cellHeight, drawWidth, m_cellHeight), fg);
                painter->setPen(bg);
            } else {
                painter->setPen(fg);
            }

            const QString glyph = cellString(cell.ch);
            painter->drawText(QPointF(x, y), glyph);

            if ((cell.attrs & 0x02) || isUrlCell) {
                const qreal underlineY = row * m_cellHeight + m_cellHeight - 1;
                painter->drawLine(QPointF(x, underlineY),
                                  QPointF(x + drawWidth, underlineY));
            }

            if (isWide) {
                ++col;
            }
        }
    }

    const int cx = m_session->cursorX();
    const int cy = m_session->cursorY();
    if (m_cursorVisible
        && (!m_cursorBlink || m_cursorBlinkVisible)
        && cx >= 0 && cy >= 0 && cx < cols && cy < rows) {
        const QColor cursorColor(m_defaultFg.red(), m_defaultFg.green(), m_defaultFg.blue(), 140);
        const qreal x0 = cx * m_cellWidth;
        const qreal y0 = cy * m_cellHeight;

        switch (m_cursorStyle) {
        case 1:
            painter->fillRect(QRectF(x0, y0, 2.0, m_cellHeight), cursorColor);
            break;
        case 2:
            painter->fillRect(QRectF(x0, y0 + m_cellHeight - 2.0, m_cellWidth, 2.0), cursorColor);
            break;
        default:
            painter->fillRect(QRectF(x0, y0, m_cellWidth, m_cellHeight), cursorColor);
            break;
        }
    }
}
