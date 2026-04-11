#include "PierTerminalGrid.h"
#include "PierTerminalSession.h"

#include "pier_terminal.h"

#include <QFontMetricsF>
#include <QPainter>

namespace {

// Minimal 16-color ANSI palette. Enough to render anything shells
// typically spit out; full 256-color + truecolor is handled below
// via the PierCell encoding.
QColor ansiIndexToColor(uint8_t idx, const QColor &defaultFg)
{
    static const QColor basic[16] = {
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
    if (idx < 16) {
        return basic[idx];
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

QColor cellForeground(const PierCell &c, const QColor &defaultFg)
{
    switch (c.fg_kind) {
    case 0:  return defaultFg;
    case 1:  return ansiIndexToColor(c.fg_r, defaultFg);
    case 2:  return QColor(c.fg_r, c.fg_g, c.fg_b);
    default: return defaultFg;
    }
}

QColor cellBackground(const PierCell &c, const QColor &defaultBg)
{
    switch (c.bg_kind) {
    case 0:  return defaultBg;
    case 1:  return ansiIndexToColor(c.bg_r, defaultBg);
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
            const QColor bg = cellBackground(c, m_defaultBg);
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
            const QColor fg = cellForeground(c, m_defaultFg);
            painter->setPen(fg);

            // Reverse video swaps fg/bg at paint time.
            if (c.attrs & 0x04 /* reverse */) {
                const QColor bg = cellBackground(c, m_defaultBg);
                painter->fillRect(
                    QRectF(col * m_cellWidth, row * m_cellHeight, m_cellWidth, m_cellHeight),
                    fg);
                painter->setPen(bg);
            }

            const QChar glyph(static_cast<char32_t>(c.ch));
            const qreal x = col * m_cellWidth;
            const qreal y = row * m_cellHeight + m_ascent;
            painter->drawText(QPointF(x, y), QString(glyph));

            if (c.attrs & 0x02 /* underline */) {
                const qreal uy = row * m_cellHeight + m_cellHeight - 1;
                painter->drawLine(QPointF(x, uy), QPointF(x + m_cellWidth, uy));
            }
        }
    }

    // Cursor block — simple filled rectangle at cursor position.
    // M2b ships non-blinking; a QTimer-driven blink animation lands
    // in v1.1. We draw it last so it's on top of any cell that
    // happens to share the cursor position.
    const int cx = m_session->cursorX();
    const int cy = m_session->cursorY();
    if (cx >= 0 && cy >= 0 && cx < cols && cy < rows) {
        painter->fillRect(
            QRectF(cx * m_cellWidth, cy * m_cellHeight, m_cellWidth, m_cellHeight),
            QColor(m_defaultFg.red(), m_defaultFg.green(), m_defaultFg.blue(), 140));
    }
}
