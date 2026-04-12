// ─────────────────────────────────────────────────────────
// PierTerminalGrid — monospace cell renderer for a terminal
// ─────────────────────────────────────────────────────────
//
// A QQuickPaintedItem that snapshots a PierTerminalSession on every
// repaint and draws the grid with QPainter. Keeps things simple for
// M2b v1: cell-by-cell drawText calls. This costs roughly
// rows*cols drawText per frame (≈ 4800 for a typical 120x40 grid),
// which QPainter handles at 60fps comfortably.
//
// Longer term, the natural upgrade is:
//   1. Batch runs of same-attribute cells into single drawText calls.
//   2. Replace with a QSGNode-based implementation that builds a
//      glyph atlas and emits a single textured-quad node per frame.
//
// Both upgrades are local to this file — none of the QML, session,
// or Rust layers know what rendering strategy we use.
//
// Cell size is derived from the current QFont at paint time using
// QFontMetricsF. When the font changes (Theme switch, DPI change,
// Settings panel) the item just repaints — no cached pixel positions
// to invalidate.

#pragma once

#include <QColor>
#include <QFont>
#include <QList>
#include <QQuickPaintedItem>
#include <QSize>
#include <QTimer>
#include <qqml.h>

// Qt moc registers Q_PROPERTY(PierTerminalSession *) as a meta-type,
// which requires the class to be complete. A forward-declare would
// compile for plain C++ but fails static_assert in qmetatype.h.
#include "PierTerminalSession.h"

class PierTerminalGrid : public QQuickPaintedItem
{
    Q_OBJECT
    QML_NAMED_ELEMENT(PierTerminalGrid)

    Q_PROPERTY(PierTerminalSession *session READ session WRITE setSession NOTIFY sessionChanged FINAL)
    Q_PROPERTY(QFont font READ font WRITE setFont NOTIFY fontChanged FINAL)
    Q_PROPERTY(QColor defaultForeground READ defaultForeground WRITE setDefaultForeground NOTIFY defaultForegroundChanged FINAL)
    Q_PROPERTY(QColor defaultBackground READ defaultBackground WRITE setDefaultBackground NOTIFY defaultBackgroundChanged FINAL)
    Q_PROPERTY(bool isDarkTheme READ isDarkTheme WRITE setIsDarkTheme NOTIFY isDarkThemeChanged FINAL)
    Q_PROPERTY(QList<QColor> paletteColors READ paletteColors WRITE setPaletteColors NOTIFY paletteColorsChanged FINAL)

    // Cursor appearance: 0 = Block, 1 = Beam, 2 = Underline
    Q_PROPERTY(int cursorStyle READ cursorStyle WRITE setCursorStyle NOTIFY cursorStyleChanged FINAL)
    Q_PROPERTY(bool cursorBlink READ cursorBlink WRITE setCursorBlink NOTIFY cursorBlinkChanged FINAL)

    // Exposed for QML to lay out the containing view. Changes with
    // the font and with the current session cell count.
    Q_PROPERTY(qreal cellWidth READ cellWidth NOTIFY metricsChanged FINAL)
    Q_PROPERTY(qreal cellHeight READ cellHeight NOTIFY metricsChanged FINAL)

public:
    explicit PierTerminalGrid(QQuickItem *parent = nullptr);

    PierTerminalSession *session() const { return m_session; }
    void setSession(PierTerminalSession *s);

    QFont font() const { return m_font; }
    void setFont(const QFont &f);

    QColor defaultForeground() const { return m_defaultFg; }
    void setDefaultForeground(const QColor &c);

    QColor defaultBackground() const { return m_defaultBg; }
    void setDefaultBackground(const QColor &c);

    bool isDarkTheme() const { return m_isDarkTheme; }
    void setIsDarkTheme(bool dark);

    QList<QColor> paletteColors() const { return m_palette; }
    void setPaletteColors(const QList<QColor> &colors);

    int cursorStyle() const { return m_cursorStyle; }
    void setCursorStyle(int style);

    bool cursorBlink() const { return m_cursorBlink; }
    void setCursorBlink(bool blink);

    qreal cellWidth() const { return m_cellWidth; }
    qreal cellHeight() const { return m_cellHeight; }

    Q_INVOKABLE QString urlAt(qreal x, qreal y) const;

    void paint(QPainter *painter) override;

public slots:
    // Ask the session to resize to whatever fits our current
    // geometry + cell metrics. QML calls this on geometry change.
    void fitToViewport();

signals:
    void sessionChanged();
    void fontChanged();
    void defaultForegroundChanged();
    void defaultBackgroundChanged();
    void isDarkThemeChanged();
    void paletteColorsChanged();
    void cursorStyleChanged();
    void cursorBlinkChanged();
    void metricsChanged();

protected:
    void geometryChange(const QRectF &newGeometry, const QRectF &oldGeometry) override;

private slots:
    void onSessionGridChanged();

private:
    void recomputeMetrics();

    PierTerminalSession *m_session = nullptr;
    QFont m_font;
    QColor m_defaultFg = Qt::white;
    QColor m_defaultBg = Qt::transparent;
    bool m_isDarkTheme = true;
    QList<QColor> m_palette; // 16 ANSI colors from QML; empty = use built-in
    int m_cursorStyle = 0;   // 0=Block, 1=Beam, 2=Underline
    bool m_cursorBlink = true;
    bool m_cursorVisible = true;
    QTimer m_blinkTimer;
    qreal m_cellWidth = 0;
    qreal m_cellHeight = 0;
    qreal m_ascent = 0;
};
