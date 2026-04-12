#include "PierTerminalGrid.h"
#include "PierTerminalSession.h"

#include "pier_terminal.h"

#include <QFontMetricsF>
#include <QPainter>
#include <QRegularExpression>

namespace {

// Minimal 16-color ANSI palette. When a custom palette is supplied
// (from QML via PierTerminalGrid::paletteColors), use it directly.
// Otherwise fall back to the built-in dark/light palettes.
QColor ansiIndexToColor(uint8_t idx, const QColor &defaultFg, bool isDark,
                        const QList<QColor> &palette)
{
    if (idx < 16) {
        // Custom palette takes priority.
        if (palette.size() >= 16)
            return palette[idx];
        if (isDark) {
            static const QColor darkPalette[16] = {
                QColor(0x00, 0x00, 0x00), // 0  black
                QColor(0xCD, 0x00, 0x00), // 1  red
                QColor(0x00, 0xCD, 0x00), // 2  green
                QColor(0xCD, 0xCD, 0x00), // 3  yellow
                QColor(0x3B, 0x78, 0xFF), // 4  blue  (tuned for dark theme)
                QColor(0xCD, 0x00, 0xCD), // 5  magenta
                QColor(0x00, 0xCD, 0xCD), // 6  cyan
                QColor(0xE5, 0xE5, 0xE5), // 7  white
                QColor(0x7F, 0x7F, 0x7F), // 8  bright black
                QColor(0xFF, 0x00, 0x00), // 9  bright red
                QColor(0x00, 0xFF, 0x00), // 10 bright green
                QColor(0xFF, 0xFF, 0x00), // 11 bright yellow
                QColor(0x5C, 0x5C, 0xFF), // 12 bright blue
                QColor(0xFF, 0x00, 0xFF), // 13 bright magenta
                QColor(0x00, 0xFF, 0xFF), // 14 bright cyan
                QColor(0xFF, 0xFF, 0xFF), // 15 bright white
            };
            return darkPalette[idx];
        } else {
            static const QColor lightPalette[16] = {
                QColor(0x00, 0x00, 0x00), // 0  black
                QColor(0xCD, 0x00, 0x00), // 1  red
                QColor(0x00, 0xA0, 0x00), // 2  green
                QColor(0xA0, 0x70, 0x00), // 3  yellow
                QColor(0x00, 0x00, 0xEE), // 4  blue
                QColor(0xCD, 0x00, 0xCD), // 5  magenta
                QColor(0x00, 0xA0, 0xA0), // 6  cyan
                QColor(0x66, 0x66, 0x66), // 7  white (darker for light bg)
                QColor(0x55, 0x55, 0x55), // 8  bright black
                QColor(0xFF, 0x00, 0x00), // 9  bright red
                QColor(0x00, 0xCD, 0x00), // 10 bright green
                QColor(0xCD, 0xCD, 0x00), // 11 bright yellow
                QColor(0x5C, 0x5C, 0xFF), // 12 bright blue
                QColor(0xFF, 0x00, 0xFF), // 13 bright magenta
                QColor(0x00, 0xCD, 0xCD), // 14 bright cyan
                QColor(0x44, 0x44, 0x44), // 15 bright white (darker for light bg)
            };
            return lightPalette[idx];
        }
    }
    // 256-color cube + grayscale ramp. Good-enough approximations;
    // the exact xterm palette has subtle variations that no one
    // will notice during normal shell usage.
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

} // namespace

PierTerminalGrid::PierTerminalGrid(QQuickItem *parent)
    : QQuickPaintedItem(parent)
{
    // Start with a reasonable fallback font. QML typically overrides
    // this via the `font` property before the first frame.
    m_font = QFont("JetBrains Mono");
    m_font.setPointSize(13);
    m_font.setStyleHint(QFont::Monospace);
    recomputeMetrics();

    // QPainter + antialiasing is the default but cell alignment is
    // critical — we want subpixel positions snapped to integer
    // pixels so glyph edges don't shimmer on repaint.
    setAntialiasing(true);
    // Children don't need to paint over us; the item paints the
    // entire rect every frame.
    setFlag(ItemHasContents, true);

    // Cursor blink timer — 530ms matches typical terminal blink rate.
    m_blinkTimer.setInterval(530);
    connect(&m_blinkTimer, &QTimer::timeout, this, [this]() {
        m_cursorVisible = !m_cursorVisible;
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
        m_cursorVisible = true;
        m_blinkTimer.start();
    } else {
        m_blinkTimer.stop();
        m_cursorVisible = true;
        update();
    }
    emit cursorBlinkChanged();
}

void PierTerminalGrid::geometryChange(const QRectF &newGeometry, const QRectF &oldGeometry)
{
    QQuickPaintedItem::geometryChange(newGeometry, oldGeometry);
    if (newGeometry.size() != oldGeometry.size()) {
        // Defer the resize to a slot so we don't re-enter scene graph
        // from inside a geometry callback. fitToViewport uses current
        // cell metrics so it's correct as soon as Qt finishes the
        // geometry event.
        fitToViewport();
    }
}

void PierTerminalGrid::onSessionGridChanged()
{
    // The session updated its snapshot on the main thread already.
    // All we have to do is ask Qt to repaint; Qt coalesces multiple
    // update() calls between frames into a single paint() call.
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
        m_session->resize(cols, rows);
        emit metricsChanged();
    }
}

void PierTerminalGrid::recomputeMetrics()
{
    const QFontMetricsF fm(m_font);
    // horizontalAdvance("M") gives the width of a typical wide
    // monospace glyph on every platform. Use the average character
    // width as a tiebreaker for fonts whose 'M' is narrower than
    // the nominal monospace width (rare but possible with variable
    // weight fonts).
    const qreal mWidth = fm.horizontalAdvance(QChar('M'));
    m_cellWidth = mWidth > 0 ? mWidth : fm.averageCharWidth();
    m_cellHeight = fm.height();
    m_ascent = fm.ascent();
}

void PierTerminalGrid::paint(QPainter *painter)
{
    // Fill the whole item background so the cell draws below don't
    // need to clear individual cells themselves.
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

    // First pass: draw every non-default background cell. Doing all
    // backgrounds before any glyphs lets QPainter batch without the
    // glyph runs fighting for state.
    for (int row = 0; row < rows; ++row) {
        for (int col = 0; col < cols; ++col) {
            const PierCell &c = cells[row * cols + col];
            if (c.bg_kind == 0) continue;
            const QColor bg = cellBackground(c, m_defaultBg, m_isDarkTheme, m_palette);
            painter->fillRect(
                QRectF(col * m_cellWidth, row * m_cellHeight, m_cellWidth, m_cellHeight),
                bg);
        }
    }

    // Second pass: glyphs.
    for (int row = 0; row < rows; ++row) {
        for (int col = 0; col < cols; ++col) {
            const PierCell &c = cells[row * cols + col];
            if (c.ch == 0 || c.ch == static_cast<uint32_t>(' ')) continue;
            const QColor fg = cellForeground(c, m_defaultFg, m_isDarkTheme, m_palette);
            painter->setPen(fg);

            // Determine if this is a wide (CJK) character that spans 2 cells.
            // Wide chars are followed by a placeholder cell with ch == 0.
            bool isWide = false;
            if (col + 1 < cols) {
                const PierCell &next = cells[row * cols + col + 1];
                // A wide char is detected when:
                // 1. The codepoint is in CJK ranges, AND
                // 2. The next cell is a zero-char placeholder
                const uint32_t cp = c.ch;
                if (next.ch == 0) {
                    // CJK Unified Ideographs and common fullwidth ranges
                    if ((cp >= 0x1100 && cp <= 0x115F) ||    // Hangul Jamo
                        cp == 0x2329 || cp == 0x232A ||      // Angle brackets
                        (cp >= 0x2E80 && cp <= 0x303E) ||    // CJK Radicals, Kangxi
                        (cp >= 0x3040 && cp <= 0x33BF) ||    // Hiragana, Katakana, CJK compat
                        (cp >= 0x3400 && cp <= 0x4DBF) ||    // CJK Extension A
                        (cp >= 0x4E00 && cp <= 0x9FFF) ||    // CJK Unified Ideographs
                        (cp >= 0xA000 && cp <= 0xA4CF) ||    // Yi
                        (cp >= 0xAC00 && cp <= 0xD7AF) ||    // Hangul Syllables
                        (cp >= 0xF900 && cp <= 0xFAFF) ||    // CJK Compatibility Ideographs
                        (cp >= 0xFE10 && cp <= 0xFE6F) ||    // CJK Compat Forms, Halfwidth
                        (cp >= 0xFF01 && cp <= 0xFF60) ||    // Fullwidth Latin
                        (cp >= 0xFFE0 && cp <= 0xFFE6) ||    // Fullwidth Signs
                        (cp >= 0x20000 && cp <= 0x2FFFF) ||  // CJK Extension B..
                        (cp >= 0x30000 && cp <= 0x3FFFF))    // CJK Extension G..
                    {
                        isWide = true;
                    }
                }
            }

            const qreal drawWidth = isWide ? m_cellWidth * 2.0 : m_cellWidth;

            // Reverse video swaps fg/bg at paint time.
            if (c.attrs & 0x04 /* reverse */) {
                const QColor bg = cellBackground(c, m_defaultBg, m_isDarkTheme, m_palette);
                painter->fillRect(
                    QRectF(col * m_cellWidth, row * m_cellHeight, drawWidth, m_cellHeight),
                    fg);
                painter->setPen(bg);
            }

            const char32_t cp = static_cast<char32_t>(c.ch);
            const QString glyph = QString::fromUcs4(&cp, 1);
            const qreal x = col * m_cellWidth;
            const qreal y = row * m_cellHeight + m_ascent;
            painter->drawText(QPointF(x, y), glyph);

            if (c.attrs & 0x02 /* underline */) {
                const qreal uy = row * m_cellHeight + m_cellHeight - 1;
                painter->drawLine(QPointF(x, uy), QPointF(x + drawWidth, uy));
            }

            // Skip the trailing placeholder cell for wide characters
            if (isWide) {
                ++col;
            }
        }
    }

    // Cursor — drawn last so it's on top of any cell content.
    // Supports Block, Beam, and Underline styles with optional blink.
    const int cx = m_session->cursorX();
    const int cy = m_session->cursorY();
    if (cx >= 0 && cy >= 0 && cx < cols && cy < rows && m_cursorVisible) {
        const QColor cursorColor(m_defaultFg.red(), m_defaultFg.green(), m_defaultFg.blue(), 140);
        const qreal x0 = cx * m_cellWidth;
        const qreal y0 = cy * m_cellHeight;

        switch (m_cursorStyle) {
        case 1: // Beam
            painter->fillRect(QRectF(x0, y0, 2.0, m_cellHeight), cursorColor);
            break;
        case 2: // Underline
            painter->fillRect(QRectF(x0, y0 + m_cellHeight - 2.0, m_cellWidth, 2.0), cursorColor);
            break;
        default: // Block
            painter->fillRect(QRectF(x0, y0, m_cellWidth, m_cellHeight), cursorColor);
            break;
        }
    }
}

QString PierTerminalGrid::urlAt(qreal x, qreal y) const
{
    if (!m_session || m_cellWidth <= 0 || m_cellHeight <= 0) {
        return QString();
    }

    int cols = 0;
    int rows = 0;
    const PierCell *cells = m_session->rawCells(&cols, &rows);
    if (!cells || cols <= 0 || rows <= 0) {
        return QString();
    }

    const int col = static_cast<int>(x / m_cellWidth);
    const int row = static_cast<int>(y / m_cellHeight);

    if (col < 0 || col >= cols || row < 0 || row >= rows) {
        return QString();
    }

    // Extract the full text of the line
    QString lineText;
    lineText.reserve(cols);
    for (int c = 0; c < cols; ++c) {
        uint32_t ch = cells[row * cols + c].ch;
        if (ch == 0) {
            lineText.append(QLatin1Char(' '));
        } else {
            lineText.append(QChar(static_cast<char32_t>(ch)));
        }
    }

    // Match HTTP/HTTPS URLs flexibly
    // Simplistic regex matching http:// or https:// followed by non-whitespace/quotes/brackets
    static const QRegularExpression urlRegex("https?://[^\\s\"'<>]+");

    QRegularExpressionMatchIterator i = urlRegex.globalMatch(lineText);
    while (i.hasNext()) {
        QRegularExpressionMatch match = i.next();
        if (col >= match.capturedStart() && col < match.capturedEnd()) {
            return match.captured(0);
        }
    }

    return QString();
}
