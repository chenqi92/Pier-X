import { FileText, FolderTree, Plus, Server, SquareTerminal, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
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

export default function TabBar({ onNewTab }: Props) {
  const { t } = useI18n();
  const { tabs, activeTabId, setActiveTab, closeTab, closeOtherTabs, setTabColor, moveTab } = useTabStore();
  const scrollRef = useRef<HTMLDivElement>(null);
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number; tabId: string } | null>(null);
  const [draggedTabId, setDraggedTabId] = useState<string | null>(null);
  const [dropTarget, setDropTarget] = useState<{ tabId: string; edge: "before" | "after" } | null>(null);

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

  return (
    <div className="tabbar" ref={scrollRef}>
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
