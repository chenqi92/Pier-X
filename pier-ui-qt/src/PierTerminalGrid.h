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
#include <QPoint>
#include <QQuickPaintedItem>
#include <QSize>
#include <QString>
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
    Q_PROPERTY(QColor selectionBackground READ selectionBackground WRITE setSelectionBackground NOTIFY selectionAppearanceChanged FINAL)
    Q_PROPERTY(QColor linkForeground READ linkForeground WRITE setLinkForeground NOTIFY linkColorsChanged FINAL)
    Q_PROPERTY(QColor linkHoverForeground READ linkHoverForeground WRITE setLinkHoverForeground NOTIFY linkColorsChanged FINAL)
    Q_PROPERTY(bool hasSelection READ hasSelection NOTIFY selectionChanged FINAL)
    Q_PROPERTY(QString hoveredUrl READ hoveredUrl NOTIFY hoveredUrlChanged FINAL)

    // Cursor appearance: 0 = Block, 1 = Beam, 2 = Underline
    Q_PROPERTY(int cursorStyle READ cursorStyle WRITE setCursorStyle NOTIFY cursorStyleChanged FINAL)
    Q_PROPERTY(bool cursorBlink READ cursorBlink WRITE setCursorBlink NOTIFY cursorBlinkChanged FINAL)
    Q_PROPERTY(bool cursorVisible READ cursorVisible WRITE setCursorVisible NOTIFY cursorVisibleChanged FINAL)

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

    QColor selectionBackground() const { return m_selectionBg; }
    void setSelectionBackground(const QColor &color);

    QColor linkForeground() const { return m_linkFg; }
    void setLinkForeground(const QColor &color);

    QColor linkHoverForeground() const { return m_linkHoverFg; }
    void setLinkHoverForeground(const QColor &color);

    bool hasSelection() const;
    QString hoveredUrl() const { return m_hoveredUrl; }

    int cursorStyle() const { return m_cursorStyle; }
    void setCursorStyle(int style);

    bool cursorBlink() const { return m_cursorBlink; }
    void setCursorBlink(bool blink);

    bool cursorVisible() const { return m_cursorVisible; }
    void setCursorVisible(bool visible);

    qreal cellWidth() const { return m_cellWidth; }
    qreal cellHeight() const { return m_cellHeight; }

    Q_INVOKABLE QString urlAt(qreal x, qreal y) const;
    Q_INVOKABLE void updateHoveredLink(qreal x, qreal y);
    Q_INVOKABLE void clearHoveredUrl();
    Q_INVOKABLE void beginSelection(qreal x, qreal y);
    Q_INVOKABLE void updateSelection(qreal x, qreal y);
    Q_INVOKABLE void endSelection();
    Q_INVOKABLE void clearSelection();
    Q_INVOKABLE bool selectWordAt(qreal x, qreal y);
    Q_INVOKABLE void selectAll();
    Q_INVOKABLE QString selectedText() const;

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
    void selectionAppearanceChanged();
    void linkColorsChanged();
    void selectionChanged();
    void hoveredUrlChanged();
    void cursorStyleChanged();
    void cursorBlinkChanged();
    void cursorVisibleChanged();
    void metricsChanged();

protected:
    void geometryChange(const QRectF &newGeometry, const QRectF &oldGeometry) override;

private slots:
    void onSessionGridChanged();

private:
    struct UrlMatch {
        int row = -1;
        int startCol = -1;
        int endCol = -1;
        QString url;

        bool isValid() const { return row >= 0 && startCol >= 0 && endCol > startCol && !url.isEmpty(); }
        bool contains(int targetRow, int targetCol) const
        {
            return targetRow == row && targetCol >= startCol && targetCol < endCol;
        }
    };

    void recomputeMetrics();
    QPoint cellAt(qreal x, qreal y, int cols, int rows, bool clampToBounds) const;
    UrlMatch hoveredMatchAt(qreal x, qreal y) const;
    UrlMatch urlMatchAt(int row, int col, const PierCell *cells, int cols, int rows) const;
    QPoint normalizedSelectionStart() const;
    QPoint normalizedSelectionEnd() const;
    bool isCellSelected(int row, int col) const;
    bool isWideLeadingCell(const PierCell *cells, int row, int col, int cols) const;
    QString lineText(const PierCell *cells, int row, int cols) const;
    QString selectedRowText(const PierCell *cells, int row, int cols, int startCol, int endCol) const;
    void setHoveredUrlMatch(const UrlMatch &match);

    PierTerminalSession *m_session = nullptr;
    QFont m_font;
    QColor m_defaultFg = Qt::white;
    QColor m_defaultBg = Qt::transparent;
    bool m_isDarkTheme = true;
    QList<QColor> m_palette; // 16 ANSI colors from QML; empty = use built-in
    QColor m_selectionBg = QColor(53, 116, 240, 46);
    QColor m_linkFg = QColor(53, 116, 240);
    QColor m_linkHoverFg = QColor(79, 138, 255);
    int m_cursorStyle = 0;   // 0=Block, 1=Beam, 2=Underline
    bool m_cursorBlink = true;
    bool m_cursorVisible = true;
    bool m_cursorBlinkVisible = true;
    QTimer m_blinkTimer;
    qreal m_cellWidth = 0;
    qreal m_cellHeight = 0;
    qreal m_ascent = 0;
    QPoint m_selectionAnchor = QPoint(-1, -1);
    QPoint m_selectionExtent = QPoint(-1, -1);
    bool m_selectionActive = false;
    UrlMatch m_hoveredMatch;
    QString m_hoveredUrl;
};
