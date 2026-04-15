import { ChevronLeft, ChevronRight, Plus, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useI18n } from "../i18n/useI18n";
import { TAB_COLORS } from "../lib/types";
import { useTabStore } from "../stores/useTabStore";
import ContextMenu, { type ContextMenuItem } from "../components/ContextMenu";

type Props = {
  onNewTab: () => void;
};

export default function TabBar({ onNewTab }: Props) {
  const { t } = useI18n();
  const { tabs, activeTabId, setActiveTab, closeTab, closeOtherTabs, setTabColor, moveTab } = useTabStore();
  const scrollRef = useRef<HTMLDivElement>(null);
  const [canScrollLeft, setCanScrollLeft] = useState(false);
  const [canScrollRight, setCanScrollRight] = useState(false);
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number; tabId: string } | null>(null);
  const [draggedTabId, setDraggedTabId] = useState<string | null>(null);
  const [dropTarget, setDropTarget] = useState<{ tabId: string; edge: "before" | "after" } | null>(null);

  const updateScrollState = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    setCanScrollLeft(el.scrollLeft > 1);
    setCanScrollRight(el.scrollLeft + el.clientWidth < el.scrollWidth - 1);
  }, []);

  useEffect(() => {
    updateScrollState();
    const el = scrollRef.current;
    if (!el) return;
    el.addEventListener("scroll", updateScrollState, { passive: true });
    const ro = new ResizeObserver(updateScrollState);
    ro.observe(el);
    return () => { el.removeEventListener("scroll", updateScrollState); ro.disconnect(); };
  }, [updateScrollState, tabs.length]);

  // Auto-scroll active tab into view
  useEffect(() => {
    if (!activeTabId || !scrollRef.current) return;
    const el = scrollRef.current;
    const activeEl = el.querySelector(`[data-tab-id="${activeTabId}"]`);
    if (activeEl) {
      activeEl.scrollIntoView({ behavior: "smooth", block: "nearest", inline: "nearest" });
    }
  }, [activeTabId]);

  function scrollBy(delta: number) {
    scrollRef.current?.scrollBy({ left: delta, behavior: "smooth" });
  }

  function moveDraggedTab(targetTabId: string, edge: "before" | "after") {
    if (!draggedTabId || draggedTabId === targetTabId) {
      return;
    }
    const fromIndex = tabs.findIndex((tab) => tab.id === draggedTabId);
    const targetIndex = tabs.findIndex((tab) => tab.id === targetTabId);
    if (fromIndex < 0 || targetIndex < 0) {
      return;
    }

    let nextIndex = edge === "after" ? targetIndex + 1 : targetIndex;
    if (fromIndex < nextIndex) {
      nextIndex -= 1;
    }
    if (nextIndex === fromIndex) {
      return;
    }
    moveTab(fromIndex, nextIndex);
  }

  if (tabs.length === 0) return null;

  const hasOverflow = canScrollLeft || canScrollRight;

  return (
    <div className="tabbar">
      {hasOverflow && (
        <button
          className="tabbar__arrow"
          disabled={!canScrollLeft}
          onClick={() => scrollBy(-160)}
          type="button"
        >
          <ChevronLeft size={14} />
        </button>
      )}

      <div className="tabbar__scroll" ref={scrollRef}>
        {tabs.map((tab) => {
          const isActive = tab.id === activeTabId;
          const colorDot =
            tab.tabColor >= 0 && tab.tabColor < TAB_COLORS.length
              ? TAB_COLORS[tab.tabColor].value
              : null;

          return (
            <button
              key={tab.id}
              className={[
                isActive ? "tab tab--active" : "tab",
                draggedTabId === tab.id ? "tab--dragging" : "",
                dropTarget?.tabId === tab.id && dropTarget.edge === "before" ? "tab--drop-before" : "",
                dropTarget?.tabId === tab.id && dropTarget.edge === "after" ? "tab--drop-after" : "",
              ].filter(Boolean).join(" ")}
              data-tab-id={tab.id}
              draggable
              onClick={() => setActiveTab(tab.id)}
              onContextMenu={(e) => { e.preventDefault(); e.stopPropagation(); setCtxMenu({ x: e.clientX, y: e.clientY, tabId: tab.id }); }}
              onDragStart={(event) => {
                setDraggedTabId(tab.id);
                event.dataTransfer.effectAllowed = "move";
                event.dataTransfer.setData("text/plain", tab.id);
              }}
              onDragOver={(event) => {
                if (!draggedTabId || draggedTabId === tab.id) {
                  return;
                }
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
              type="button"
            >
              {colorDot ? (
                <span
                  className="tab__color"
                  style={{ backgroundColor: colorDot }}
                />
              ) : null}
              <span className="tab__title">{tab.title}</span>
              <span
                className="tab__close"
                onClick={(e) => {
                  e.stopPropagation();
                  closeTab(tab.id);
                }}
                role="button"
                tabIndex={-1}
              >
                <X size={12} />
              </span>
            </button>
          );
        })}
      </div>

      {hasOverflow && (
        <button
          className="tabbar__arrow"
          disabled={!canScrollRight}
          onClick={() => scrollBy(160)}
          type="button"
        >
          <ChevronRight size={14} />
        </button>
      )}

      <button
        className="tabbar__add"
        onClick={onNewTab}
        title={t("New tab")}
        type="button"
      >
        <Plus size={14} />
      </button>

      {ctxMenu && (() => {
        const isMac = navigator.platform.includes("Mac");
        const mod = isMac ? "\u2318" : "Ctrl+";
        const items: ContextMenuItem[] = [
          { label: t("Close tab"), shortcut: `${mod}W`, action: () => closeTab(ctxMenu.tabId) },
          { label: t("Close others"), action: () => closeOtherTabs(ctxMenu.tabId), disabled: tabs.length <= 1 },
          { divider: true },
          ...TAB_COLORS.map((color, i) => ({
            label: `● ${color.name}`,
            action: () => setTabColor(ctxMenu.tabId, i),
          })),
          { label: t("Clear color"), action: () => setTabColor(ctxMenu.tabId, -1) },
        ];
        return <ContextMenu x={ctxMenu.x} y={ctxMenu.y} items={items} onClose={() => setCtxMenu(null)} />;
      })()}
    </div>
  );
}
