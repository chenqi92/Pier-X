import { FolderTree, Plus, Search, Server, SquareTerminal } from "lucide-react";
import { useEffect, useState } from "react";
import type { FileEntry } from "../lib/types";
import * as cmd from "../lib/commands";
import { useI18n } from "../i18n/useI18n";
import { useConnectionStore } from "../stores/useConnectionStore";

type Props = {
  onOpenLocalTerminal: (path?: string) => void;
  onConnectSaved: (index: number) => void;
  onNewConnection: () => void;
};

export default function Sidebar({ onOpenLocalTerminal, onConnectSaved, onNewConnection }: Props) {
  const { t } = useI18n();
  const [section, setSection] = useState<0 | 1>(0); // 0=Files, 1=Servers
  const [entries, setEntries] = useState<FileEntry[]>([]);
  const [browserPath, setBrowserPath] = useState("");
  const [searchText, setSearchText] = useState("");
  const { connections, refresh: refreshConnections } = useConnectionStore();
  const [serverSearch, setServerSearch] = useState("");

  // Fetch directory on path change
  useEffect(() => {
    cmd.listDirectory(browserPath || undefined).then((next) => {
      setEntries(next);
    }).catch(() => {});
  }, [browserPath]);

  // Load connections on mount
  useEffect(() => {
    refreshConnections();
  }, []);

  const filteredEntries = entries.filter((entry) => {
    if (!searchText.trim()) return true;
    return entry.name.toLowerCase().includes(searchText.toLowerCase());
  });

  const filteredConnections = connections.filter((conn) => {
    if (!serverSearch.trim()) return true;
    const q = serverSearch.toLowerCase();
    return (
      conn.name.toLowerCase().includes(q) ||
      conn.host.toLowerCase().includes(q) ||
      conn.user.toLowerCase().includes(q)
    );
  });

  const pathParts = browserPath.split(/[\\/]/).filter(Boolean);
  const parentPath = browserPath ? browserPath.replace(/[\\/][^\\/]+[\\/]?$/, "") || "/" : "";

  return (
    <aside className="sidebar">
      {/* Tab switcher */}
      <div className="sidebar__tabs">
        <button
          className={section === 0 ? "sidebar__tab sidebar__tab--active" : "sidebar__tab"}
          onClick={() => setSection(0)}
          type="button"
        >
          <FolderTree size={13} />
          {t("Files")}
        </button>
        <button
          className={section === 1 ? "sidebar__tab sidebar__tab--active" : "sidebar__tab"}
          onClick={() => setSection(1)}
          type="button"
        >
          <Server size={13} />
          {t("Servers")}
        </button>
      </div>

      {section === 0 ? (
        /* ── Files Pane ─────────────────────────────── */
        <div className="sidebar__pane">
          {/* Breadcrumb */}
          <div className="sidebar__breadcrumb">
            {browserPath ? (
              <button className="sidebar__crumb" onClick={() => setBrowserPath(parentPath)} type="button">
                ..
              </button>
            ) : null}
            {pathParts.map((part, i) => (
              <button
                key={i}
                className="sidebar__crumb"
                onClick={() =>
                  setBrowserPath(
                    pathParts.slice(0, i + 1).join("/"),
                  )
                }
                type="button"
              >
                {part}
              </button>
            ))}
          </div>

          {/* Search */}
          <div className="sidebar__search">
            <Search size={12} />
            <input
              className="sidebar__search-input"
              onChange={(e) => setSearchText(e.currentTarget.value)}
              placeholder={t("Search files…")}
              value={searchText}
            />
          </div>

          {/* File list */}
          <div className="sidebar__list">
            {filteredEntries.map((entry) => (
              <button
                key={entry.path}
                className="sidebar__file-row"
                onClick={() => {
                  if (entry.kind === "directory") {
                    setBrowserPath(entry.path);
                  }
                }}
                onDoubleClick={() => {
                  if (entry.kind === "directory") {
                    onOpenLocalTerminal(entry.path);
                  }
                }}
                type="button"
              >
                <span className={entry.kind === "directory" ? "sidebar__file-icon sidebar__file-icon--dir" : "sidebar__file-icon"}>
                  {entry.kind === "directory" ? <FolderTree size={13} /> : null}
                </span>
                <span className="sidebar__file-name">{entry.name}</span>
                <span className="sidebar__file-meta">{entry.sizeLabel}</span>
              </button>
            ))}
          </div>
        </div>
      ) : (
        /* ── Servers Pane ───────────────────────────── */
        <div className="sidebar__pane">
          <div className="sidebar__pane-header">
            <span className="sidebar__pane-label">{connections.length} {t("Connections")}</span>
            <button className="topbar__icon-btn" onClick={onNewConnection} title={t("New SSH connection")} type="button">
              <Plus size={14} />
            </button>
          </div>

          <div className="sidebar__search">
            <Search size={12} />
            <input
              className="sidebar__search-input"
              onChange={(e) => setServerSearch(e.currentTarget.value)}
              placeholder={t("Search servers…")}
              value={serverSearch}
            />
          </div>

          <div className="sidebar__list">
            {filteredConnections.map((conn) => (
              <button
                key={`${conn.index}-${conn.name}`}
                className="sidebar__server-row"
                onClick={() => onConnectSaved(conn.index)}
                type="button"
              >
                <SquareTerminal size={14} />
                <div className="sidebar__server-info">
                  <strong>{conn.name}</strong>
                  <span>{conn.user}@{conn.host}:{conn.port}</span>
                </div>
                <span className="sidebar__auth-pill">{conn.authKind}</span>
              </button>
            ))}
            {filteredConnections.length === 0 && (
              <div className="empty-note" style={{ padding: "12px 16px" }}>
                {connections.length === 0
                  ? t("New SSH connection")
                  : t("No matching commands")}
              </div>
            )}
          </div>
        </div>
      )}
    </aside>
  );
}
