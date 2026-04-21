import { ChevronLeft, ChevronRight, FileText, FolderTree, Plus, Server, SquareTerminal, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useI18n } from "../i18n/useI18n";
import { TAB_COLORS, type TabState } from "../lib/types";
import { useTabStore } from "../stores/useTabStore";
import ContextMenu, { type ContextMenuItem } from "../components/ContextMenu";

function backendIcon(backend: TabState["backend"]) {
  switch (backend) {
    case "ssh":
      return <Server className="tab-icon" size={13} />;
    case "sftp":
      return <FolderTree className="tab-icon" size={13} />;
    case "markdown":
      return <FileText className="tab-icon" size={13} />;
    default:
      return <SquareTerminal className="tab-icon" size={13} />;
  }
}

type Props = {
  onNewTab: () => void;
};

const SCROLL_STEP = 160;
const HOLD_DELAY = 220;
const HOLD_STEP_PX = 18;
const HOLD_INTERVAL = 16;

export default function TabBar({ onNewTab }: Props) {
  const { t } = useI18n();
  const {
    tabs,
    activeTabId,
    setActiveTab,
    closeTab,
    closeOtherTabs,
    closeTabsToLeft,
    closeTabsToRight,
    setTabColor,
    moveTab,
  } = useTabStore();
  const scrollRef = useRef<HTMLDivElement>(null);
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number; tabId: string } | null>(null);
  const [draggedTabId, setDraggedTabId] = useState<string | null>(null);
  const [dropTarget, setDropTarget] = useState<{ tabId: string; edge: "before" | "after" } | null>(null);
  const [overflow, setOverflow] = useState({ left: false, right: false });
  const holdTimer = useRef<number | null>(null);
  const holdInterval = useRef<number | null>(null);

  const updateOverflow = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    const { scrollLeft, scrollWidth, clientWidth } = el;
    setOverflow({
      left: scrollLeft > 0,
      right: scrollLeft + clientWidth < scrollWidth - 1,
    });
  }, []);

  // Watch container size + children changes to recompute overflow state
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    updateOverflow();
    const ro = new ResizeObserver(updateOverflow);
    ro.observe(el);
    for (const child of Array.from(el.children)) ro.observe(child);
    return () => ro.disconnect();
  }, [tabs.length, updateOverflow]);

  // Auto-scroll active tab into view
  useEffect(() => {
    if (!activeTabId || !scrollRef.current) return;
    const el = scrollRef.current;
    const activeEl = el.querySelector(`[data-tab-id="${activeTabId}"]`);
    if (activeEl) {
      activeEl.scrollIntoView({ behavior: "smooth", block: "nearest", inline: "nearest" });
    }
  }, [activeTabId]);

  function moveDraggedTab(targetTabId: string, edge: "before" | "after") {
    if (!draggedTabId || draggedTabId === targetTabId) return;
    const fromIndex = tabs.findIndex((tab) => tab.id === draggedTabId);
    const targetIndex = tabs.findIndex((tab) => tab.id === targetTabId);
    if (fromIndex < 0 || targetIndex < 0) return;

    let nextIndex = edge === "after" ? targetIndex + 1 : targetIndex;
    if (fromIndex < nextIndex) nextIndex -= 1;
    if (nextIndex === fromIndex) return;
    moveTab(fromIndex, nextIndex);
  }

  function scrollBy(delta: number) {
    scrollRef.current?.scrollBy({ left: delta, behavior: "smooth" });
  }

  const stopHoldScroll = useCallback(() => {
    if (holdTimer.current !== null) {
      window.clearTimeout(holdTimer.current);
      holdTimer.current = null;
    }
    if (holdInterval.current !== null) {
      window.clearInterval(holdInterval.current);
      holdInterval.current = null;
    }
  }, []);

  useEffect(() => stopHoldScroll, [stopHoldScroll]);

  function startHoldScroll(direction: -1 | 1) {
    stopHoldScroll();
    // Immediate step, then accelerate after a short delay
    scrollBy(direction * SCROLL_STEP);
    holdTimer.current = window.setTimeout(() => {
      holdInterval.current = window.setInterval(() => {
        const el = scrollRef.current;
        if (!el) return;
        el.scrollLeft += direction * HOLD_STEP_PX;
      }, HOLD_INTERVAL);
    }, HOLD_DELAY);
  }

  function onWheel(event: React.WheelEvent<HTMLDivElement>) {
    const el = scrollRef.current;
    if (!el) return;
    // Translate vertical wheel into horizontal scroll when there's overflow.
    if (event.deltaY !== 0 && el.scrollWidth > el.clientWidth) {
      el.scrollLeft += event.deltaY;
    }
  }

  return (
    <div className="tabbar">
      <button
        className="tabbar-arrow"
        disabled={!overflow.left}
        title={t("Scroll tabs left")}
        type="button"
        aria-label={t("Scroll tabs left")}
        onPointerDown={(e) => {
          if (e.button !== 0 || !overflow.left) return;
          e.currentTarget.setPointerCapture(e.pointerId);
          startHoldScroll(-1);
        }}
        onPointerUp={stopHoldScroll}
        onPointerLeave={stopHoldScroll}
        onPointerCancel={stopHoldScroll}
      >
        <ChevronLeft size={14} />
      </button>

      <div
        className="tabbar-scroll"
        ref={scrollRef}
        onScroll={updateOverflow}
        onWheel={onWheel}
      >
        {tabs.map((tab) => {
          const isActive = tab.id === activeTabId;
          const colorDot =
            tab.tabColor >= 0 && tab.tabColor < TAB_COLORS.length
              ? TAB_COLORS[tab.tabColor].value
              : null;

          return (
            <div
              key={tab.id}
              className={[
                "tab",
                isActive ? "active" : "",
                draggedTabId === tab.id ? "is-dragging" : "",
                dropTarget?.tabId === tab.id && dropTarget.edge === "before" ? "is-drop-before" : "",
                dropTarget?.tabId === tab.id && dropTarget.edge === "after" ? "is-drop-after" : "",
              ].filter(Boolean).join(" ")}
              data-tab-id={tab.id}
              draggable
              onClick={() => setActiveTab(tab.id)}
              onContextMenu={(e) => {
                e.preventDefault();
                e.stopPropagation();
                setCtxMenu({ x: e.clientX, y: e.clientY, tabId: tab.id });
              }}
              onDragStart={(event) => {
                setDraggedTabId(tab.id);
                event.dataTransfer.effectAllowed = "move";
                event.dataTransfer.setData("text/plain", tab.id);
              }}
              onDragOver={(event) => {
                if (!draggedTabId || draggedTabId === tab.id) return;
                event.preventDefault();
                const bounds = event.currentTarget.getBoundingClientRect();
                const edge = event.clientX - bounds.left < bounds.width / 2 ? "before" : "after";
                setDropTarget({ tabId: tab.id, edge });
              }}
              onDragLeave={(event) => {
                if (!event.currentTarget.contains(event.relatedTarget as Node | null)) {
                  setDropTarget((current) => current?.tabId === tab.id ? null : current);
                }
              }}
              onDrop={(event) => {
                event.preventDefault();
                const bounds = event.currentTarget.getBoundingClientRect();
                const edge = event.clientX - bounds.left < bounds.width / 2 ? "before" : "after";
                moveDraggedTab(tab.id, edge);
                setDraggedTabId(null);
                setDropTarget(null);
              }}
              onDragEnd={() => {
                setDraggedTabId(null);
                setDropTarget(null);
              }}
              role="button"
              tabIndex={0}
            >
              <span
                className="tab-color"
                style={colorDot ? { backgroundColor: colorDot } : undefined}
              />
              {backendIcon(tab.backend)}
              <span className="tab-title">{tab.title}</span>
              <button
                className="tab-close"
                onClick={(e) => {
                  e.stopPropagation();
                  closeTab(tab.id);
                }}
                title={t("Close tab")}
                type="button"
              >
                <X size={9} />
              </button>
            </div>
          );
        })}
      </div>

      <button
        className="tabbar-arrow"
        disabled={!overflow.right}
        title={t("Scroll tabs right")}
        type="button"
        aria-label={t("Scroll tabs right")}
        onPointerDown={(e) => {
          if (e.button !== 0 || !overflow.right) return;
          e.currentTarget.setPointerCapture(e.pointerId);
          startHoldScroll(1);
        }}
        onPointerUp={stopHoldScroll}
        onPointerLeave={stopHoldScroll}
        onPointerCancel={stopHoldScroll}
      >
        <ChevronRight size={14} />
      </button>

      <button
        className="tabbar-new"
        onClick={onNewTab}
        title={t("New tab")}
        type="button"
      >
        <Plus size={13} />
      </button>

      {ctxMenu && (() => {
        const isMac = navigator.platform.includes("Mac");
        const mod = isMac ? "⌘" : "Ctrl+";
        const idx = tabs.findIndex((tab) => tab.id === ctxMenu.tabId);
        const items: ContextMenuItem[] = [
          { label: t("Close tab"), shortcut: `${mod}W`, action: () => closeTab(ctxMenu.tabId) },
          { label: t("Close others"), action: () => closeOtherTabs(ctxMenu.tabId), disabled: tabs.length <= 1 },
          { label: t("Close to left"), action: () => closeTabsToLeft(ctxMenu.tabId), disabled: idx <= 0 },
          { label: t("Close to right"), action: () => closeTabsToRight(ctxMenu.tabId), disabled: idx < 0 || idx >= tabs.length - 1 },
          { divider: true },
          ...TAB_COLORS.map((color, i) => ({
            label: t(color.name),
            iconColor: color.value,
            action: () => setTabColor(ctxMenu.tabId, i),
          })),
          { label: t("Clear color"), iconColor: "", action: () => setTabColor(ctxMenu.tabId, -1) },
        ];
        return <ContextMenu x={ctxMenu.x} y={ctxMenu.y} items={items} onClose={() => setCtxMenu(null)} />;
      })()}
    </div>
  );
}
