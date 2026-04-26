import { Activity, Edit, FileText, History, Lock, Play, Plus, Star, Unlock, Wand2, X } from "lucide-react";
import { useMemo, useState } from "react";

import { useI18n } from "../../i18n/useI18n";
import { renderSqlTokens } from "./sqlHighlight";

/** One open query in the multi-tab editor. */
export type SqlTab = {
  id: string;
  /** Display name (file-tab style). Often a query alias or table name. */
  name: string;
  sql: string;
  /** Pulse-dot indicator next to the tab name. */
  dirty?: boolean;
};

/** One entry in the right-side history rail. */
export type SqlHistoryEntry = {
  /** Snapshot of the SQL that was run. */
  sql: string;
  /** Friendly when-string (e.g. "2m ago"). Caller formats. */
  at: string;
  rows?: number | null;
  ms?: number;
  /** True for INSERT/UPDATE/DELETE/DDL — gets the edit icon and a tint. */
  write?: boolean;
};

/** One pinned/saved query. Persists across reloads in the same
 *  per-engine localStorage bucket as `SqlHistoryEntry` so a user
 *  who clears history doesn't lose their favorites. */
export type SqlFavoriteEntry = {
  /** Stable id — used as the React key + delete target. */
  id: string;
  /** User-supplied label; defaults to a truncated SQL preview. */
  name: string;
  sql: string;
  /** Unix ms when the favorite was added. */
  savedAt: number;
};

type Props = {
  /** Single-tab fallback name used when `tabs` isn't supplied. */
  tabName?: string;
  /** Active tab's SQL — always controlled by the parent. */
  sql: string;
  /** Patches the *active* tab's SQL. */
  onChange: (next: string) => void;
  writable: boolean;
  onToggleWrite: () => void;
  /** When a write-class statement is typed, the user must retype "WRITE". */
  needsWriteConfirm: boolean;
  writeConfirm: string;
  onWriteConfirmChange: (next: string) => void;
  onRun: () => void;
  canRun: boolean;
  running: boolean;

  /** When provided, renders the multi-tab strip. The parent owns
   *  tabs / activeTabId state and handler callbacks. */
  tabs?: SqlTab[];
  activeTabId?: string;
  onActiveTabChange?: (id: string) => void;
  onAddTab?: () => void;
  onCloseTab?: (id: string) => void;

  /** When provided, renders the History side panel toggle + drawer. */
  history?: SqlHistoryEntry[];
  /** Called when a history row is clicked — typically loads it into a tab. */
  onPickHistory?: (entry: SqlHistoryEntry) => void;

  /** When provided, the Favorites button enables and renders the
   *  side drawer with pinned queries. */
  favorites?: SqlFavoriteEntry[];
  /** Pin the currently-active SQL. The editor passes the live SQL
   *  + active tab name so the panel can call `addFavorite` cleanly. */
  onAddFavorite?: (sql: string, defaultName: string) => void;
  /** Remove a pinned query by id. */
  onRemoveFavorite?: (id: string) => void;
  /** Load a pinned query into the active tab. */
  onPickFavorite?: (entry: SqlFavoriteEntry) => void;

  /** Optional EXPLAIN handler — when omitted, button hidden. */
  onExplain?: () => void;

  /** Optional Format-SQL handler. When provided, the wand button
   *  in the toolbar is enabled and clicking it asks the parent to
   *  reformat the active tab's SQL (parent owns dialect choice).
   *  When omitted, the button shows the existing "coming soon"
   *  disabled state. */
  onFormat?: () => void;
};

/**
 * SQL editor chrome. Single-tab mode (no `tabs` prop) keeps the
 * pier-x-copy visual: file-tab style header, gutter with line numbers,
 * transparent textarea over a highlighted `<pre>`. Multi-tab mode adds
 * the tab strip with new/close + a History side panel that overlays
 * the editor body.
 */
export default function DbSqlEditor({
  tabName,
  sql,
  onChange,
  writable,
  onToggleWrite,
  needsWriteConfirm,
  writeConfirm,
  onWriteConfirmChange,
  onRun,
  canRun,
  running,
  tabs,
  activeTabId,
  onActiveTabChange,
  onAddTab,
  onCloseTab,
  history,
  onPickHistory,
  favorites,
  onAddFavorite,
  onRemoveFavorite,
  onPickFavorite,
  onExplain,
  onFormat,
}: Props) {
  const { t } = useI18n();
  const lines = useMemo(() => sql.split("\n"), [sql]);
  const tokens = useMemo(() => renderSqlTokens(sql), [sql]);
  const [histOpen, setHistOpen] = useState(false);
  const [favOpen, setFavOpen] = useState(false);
  const favoritesEnabled = !!favorites && !!onAddFavorite;
  const activeTabName =
    tabs?.find((tab) => tab.id === activeTabId)?.name ?? tabName ?? "query";
  const isCurrentSqlPinned =
    !!favorites && favorites.some((f) => f.sql.trim() === sql.trim() && sql.trim() !== "");

  const isMulti = !!tabs && tabs.length > 0;

  return (
    <div className="sq">
      <div className="sq-tabs">
        {isMulti ? (
          tabs!.map((tab) => {
            const active = tab.id === activeTabId;
            return (
              <button
                key={tab.id}
                type="button"
                className={"sq-tab" + (active ? " active" : "")}
                onClick={() => onActiveTabChange?.(tab.id)}
                title={tab.name}
              >
                <FileText size={10} />
                <span className="sq-tab-name">{tab.name}</span>
                {tab.dirty && <span className="sq-tab-dot" aria-hidden />}
                {tabs!.length > 1 && onCloseTab && (
                  <span
                    className="sq-tab-x"
                    role="button"
                    aria-label={t("Close tab")}
                    onClick={(e) => {
                      e.stopPropagation();
                      onCloseTab(tab.id);
                    }}
                  >
                    <X size={8} />
                  </span>
                )}
              </button>
            );
          })
        ) : (
          <span className="sq-tab active">
            <FileText size={10} />
            <span>{tabName ?? t("query")}</span>
          </span>
        )}
        {isMulti && onAddTab && (
          <button
            type="button"
            className="sq-tab-add"
            onClick={onAddTab}
            title={t("New query")}
          >
            <Plus size={11} />
          </button>
        )}
        <span className="sq-spacer" />
        {history && history.length > 0 && (
          <button
            type="button"
            className={"sq-mini" + (histOpen ? " on" : "")}
            onClick={() => setHistOpen((v) => !v)}
            title={t("History")}
          >
            <History size={11} />
          </button>
        )}
        {favoritesEnabled ? (
          <>
            <button
              type="button"
              className="sq-mini"
              disabled={!sql.trim() || isCurrentSqlPinned}
              onClick={() => onAddFavorite?.(sql, activeTabName)}
              title={
                isCurrentSqlPinned
                  ? t("Already pinned")
                  : t("Pin this query to favorites")
              }
            >
              <Star
                size={11}
                fill={isCurrentSqlPinned ? "currentColor" : "none"}
              />
            </button>
            {favorites && favorites.length > 0 && (
              <button
                type="button"
                className={"sq-mini" + (favOpen ? " on" : "")}
                onClick={() => setFavOpen((v) => !v)}
                title={t("Favorites")}
              >
                <Star size={11} fill="currentColor" />
                <span style={{ marginLeft: 2, fontSize: "var(--size-small)" }}>
                  {favorites.length}
                </span>
              </button>
            )}
          </>
        ) : (
          <button type="button" className="sq-mini" disabled title={t("Favorites — coming soon")}>
            <Star size={11} />
          </button>
        )}
        <button
          type="button"
          className="sq-mini"
          disabled={!onFormat}
          onClick={onFormat}
          title={onFormat ? t("Format SQL") : t("Format SQL — coming soon")}
        >
          <Wand2 size={11} />
        </button>
      </div>

      <div className="sq-editor-wrap">
        <div className="sq-gutter" aria-hidden>
          {lines.map((_, i) => (
            <div key={i} className="sq-gutter-n">
              {i + 1}
            </div>
          ))}
        </div>
        <div className="sq-editor-body">
          <pre className="sq-hl" aria-hidden>
            {tokens}
            {"\n"}
          </pre>
          <textarea
            className="sq-ta"
            value={sql}
            spellCheck={false}
            onChange={(e) => onChange(e.currentTarget.value)}
            onKeyDown={(e) => {
              if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
                e.preventDefault();
                if (canRun) onRun();
              }
            }}
          />
        </div>
        {histOpen && history && (
          <div className="sq-hist">
            <div className="sq-hist-head">
              <History size={10} />
              <span>{t("HISTORY")}</span>
              <span className="sq-spacer" />
              <button
                type="button"
                className="mini-button mini-button--ghost"
                onClick={() => setHistOpen(false)}
                title={t("Close")}
              >
                <X size={10} />
              </button>
            </div>
            <div className="sq-hist-list">
              {history.map((h, i) => (
                <button
                  key={i}
                  type="button"
                  className="sq-hist-row"
                  onClick={() => onPickHistory?.(h)}
                >
                  <span className={"sq-hist-ic" + (h.write ? " w" : "")}>
                    {h.write ? <Edit size={9} /> : <Play size={9} />}
                  </span>
                  <div className="sq-hist-body">
                    <div className="sq-hist-sql">{h.sql}</div>
                    <div className="sq-hist-meta">
                      <span>{h.at}</span>
                      {h.rows != null && (
                        <>
                          <span className="sep">·</span>
                          <span>{t("{rows} rows", { rows: h.rows })}</span>
                        </>
                      )}
                      {typeof h.ms === "number" && (
                        <>
                          <span className="sep">·</span>
                          <span>{h.ms}ms</span>
                        </>
                      )}
                    </div>
                  </div>
                </button>
              ))}
            </div>
          </div>
        )}
        {favOpen && favorites && (
          <div className="sq-hist">
            <div className="sq-hist-head">
              <Star size={10} fill="currentColor" />
              <span>{t("FAVORITES")}</span>
              <span className="sq-spacer" />
              <button
                type="button"
                className="mini-button mini-button--ghost"
                onClick={() => setFavOpen(false)}
                title={t("Close")}
              >
                <X size={10} />
              </button>
            </div>
            <div className="sq-hist-list">
              {favorites.length === 0 && (
                <div
                  className="sq-hist-row"
                  style={{ color: "var(--muted)", padding: "var(--sp-3)" }}
                >
                  {t("No pinned queries yet.")}
                </div>
              )}
              {favorites.map((f) => (
                <div key={f.id} className="sq-hist-row" style={{ display: "flex", alignItems: "center" }}>
                  <button
                    type="button"
                    className="sq-hist-row"
                    style={{ flex: 1, border: 0, background: "transparent", padding: 0 }}
                    onClick={() => onPickFavorite?.(f)}
                  >
                    <span className="sq-hist-ic">
                      <Star size={9} fill="currentColor" />
                    </span>
                    <div className="sq-hist-body">
                      <div className="sq-hist-sql"><b>{f.name}</b></div>
                      <div className="sq-hist-meta">
                        <span style={{ color: "var(--muted)" }}>{f.sql}</span>
                      </div>
                    </div>
                  </button>
                  {onRemoveFavorite && (
                    <button
                      type="button"
                      className="mini-button mini-button--ghost"
                      onClick={(e) => {
                        e.stopPropagation();
                        onRemoveFavorite(f.id);
                      }}
                      title={t("Unpin")}
                    >
                      <X size={9} />
                    </button>
                  )}
                </div>
              ))}
            </div>
          </div>
        )}
      </div>

      <div className="sq-foot">
        <button
          type="button"
          className={"sq-lock" + (writable ? " on" : "")}
          onClick={onToggleWrite}
        >
          {writable ? <Unlock size={10} /> : <Lock size={10} />}
          {writable ? t("Writes unlocked") : t("Read-only")}
        </button>
        <span className="sq-foot-hint">
          {writable ? t("DML/DDL will execute.") : t("Unlock to run INSERT/UPDATE/DELETE.")}
        </span>
        {needsWriteConfirm && writable && (
          <input
            className="sq-confirm"
            value={writeConfirm}
            onChange={(e) => onWriteConfirmChange(e.currentTarget.value)}
            placeholder={t("Type WRITE to confirm")}
          />
        )}
        <span className="sq-spacer" />
        <span className="sq-shortcut">⌘↵ {t("run")}</span>
        {onExplain && (
          <button
            type="button"
            className="btn is-ghost is-compact"
            disabled={!canRun}
            onClick={onExplain}
            title={t("EXPLAIN selected query")}
          >
            <Activity size={10} /> {t("EXPLAIN")}
          </button>
        )}
        <button
          type="button"
          className="btn is-primary is-compact"
          disabled={!canRun}
          onClick={onRun}
        >
          <Play size={10} /> {running ? t("Running...") : t("Run")}
        </button>
      </div>
    </div>
  );
}
