import {
  ChevronDown,
  ChevronRight,
  Database,
  Eye,
  FolderTree,
  FunctionSquare,
  Search,
  Table,
  X,
} from "lucide-react";
import { useMemo, useRef, useState, type MouseEvent as ReactMouseEvent } from "react";

import { useI18n } from "../../i18n/useI18n";
import ContextMenu, { type ContextMenuItem } from "../ContextMenu";

export type DbSchemaNode = {
  id: string;
  label: string;
  count?: number | null;
  /** Optional small muted label shown next to the node icon —
   *  e.g. `"FN"` / `"PR"` for routines so the tree can mix
   *  procedures + functions in one folder without losing the
   *  discriminator. Empty / omitted = no badge. */
  badge?: string | null;
  /** Optional tooltip surfaced on hover. Used for the `title=`
   *  attribute on the row button so the table-meta enrichment
   *  (engine, on-disk size, last-update timestamp) can be read
   *  without expanding the row. */
  tooltip?: string | null;
};

export type DbSchemaDatabase = {
  name: string;
  current: boolean;
  tables: DbSchemaNode[];
  /** Views defined in this database. Rendered under their own
   *  collapsible "Views" folder when present. Optional so the
   *  PostgreSQL / SQLite panels — which haven't wired view
   *  enumeration yet — keep working without a reshape. */
  views?: DbSchemaNode[];
  /** Stored procedures + functions defined in this database.
   *  Same optionality story as `views`. */
  routines?: DbSchemaNode[];
  /** Optional schema list for engines that have a schema layer
   *  (PostgreSQL). When provided, the tree renders a sibling
   *  "Schemas" group with `activeSchema` highlighted; clicking
   *  another schema fires `onSelectSchema`. Engines without
   *  schemas (MySQL / SQLite) leave both fields undefined. */
  schemas?: string[];
  /** Active schema name — only meaningful when `schemas` is set. */
  activeSchema?: string;
};

/** Right-click action callbacks. Each one is optional — if a panel
 *  doesn't wire it, the corresponding menu item is hidden rather
 *  than disabled. The "tables" arg is always an array so single +
 *  multi-select share one signature; for single-table actions it's
 *  a one-element array.
 *
 *  All callbacks are fire-and-forget from the tree's perspective —
 *  the panel decides whether to show a confirmation dialog. */
export type DbSchemaActions = {
  /** Tree-root or empty-area right click. */
  onCreateDatabase?: () => void;
  onImportSql?: () => void;
  /** Right-click on a database row. */
  onRefreshDatabase?: (db: string) => void;
  onExportDatabase?: (db: string) => void;
  onDropDatabase?: (db: string) => void;
  /** Right-click on a table row (or on the multi-select). */
  onCopyTableName?: (db: string, tables: string[]) => void;
  onExportTables?: (db: string, tables: string[]) => void;
  onTruncateTables?: (db: string, tables: string[]) => void;
  onDropTables?: (db: string, tables: string[]) => void;
};

type Props = {
  databases: DbSchemaDatabase[];
  selectedTableId: string | null;
  onSelectTable: (databaseName: string, node: DbSchemaNode) => void;
  onSelectDatabase?: (name: string) => void;
  /** Fired when the user clicks a non-active schema row. The
   *  panel re-runs `postgresBrowse` with the new schema. */
  onSelectSchema?: (databaseName: string, schema: string) => void;
  /** Optional right-click actions. Engines opt in by passing the
   *  callbacks they support. */
  actions?: DbSchemaActions;
};

type MenuState = {
  x: number;
  y: number;
  items: ContextMenuItem[];
};

/**
 * Left-rail schema tree. Renders the active database's tables
 * (always), plus optional Views and Routines folders when the
 * caller supplies the corresponding arrays. Non-current
 * databases collapse to a stub row the user can click to switch.
 */
export default function DbSchemaTree({
  databases,
  selectedTableId,
  onSelectTable,
  onSelectDatabase,
  onSelectSchema,
  actions,
}: Props) {
  const { t } = useI18n();
  const [openDb, setOpenDb] = useState<Record<string, boolean>>(() =>
    Object.fromEntries(databases.filter((d) => d.current).map((d) => [d.name, true])),
  );
  const [openTables, setOpenTables] = useState<Record<string, boolean>>(() =>
    Object.fromEntries(databases.filter((d) => d.current).map((d) => [d.name, true])),
  );
  // Views / Routines start collapsed by default — they're new
  // surfaces, and most users come to the tree to find tables.
  // Click to expand once and the state persists for the session.
  const [openViews, setOpenViews] = useState<Record<string, boolean>>({});
  const [openRoutines, setOpenRoutines] = useState<Record<string, boolean>>({});
  const [q, setQ] = useState("");
  /** Multi-select for table rows. Holds DbSchemaNode.id values. The
   *  panel's `selectedTableId` is the "primary" selection (always
   *  reflected in the schema tree's `sel` class); the multi-select
   *  is purely local — it powers right-click "Export selected" without
   *  bothering the panel until the user asks. */
  const [multiSel, setMultiSel] = useState<Set<string>>(new Set());
  /** Anchor for shift-click range selection. Cleared whenever the
   *  user clicks without any modifier key. */
  const anchorRef = useRef<{ db: string; index: number } | null>(null);
  const [menu, setMenu] = useState<MenuState | null>(null);

  const qLower = q.trim().toLowerCase();
  const visible = useMemo(() => {
    if (!qLower) return databases;
    return databases.map((db) => ({
      ...db,
      tables: db.tables.filter((t) => t.label.toLowerCase().includes(qLower)),
      views: db.views?.filter((v) => v.label.toLowerCase().includes(qLower)),
      routines: db.routines?.filter((r) => r.label.toLowerCase().includes(qLower)),
    }));
  }, [databases, qLower]);

  const toggleDb = (name: string) =>
    setOpenDb((prev) => ({ ...prev, [name]: !prev[name] }));
  const toggleTables = (name: string) =>
    setOpenTables((prev) => ({ ...prev, [name]: !prev[name] }));
  const toggleViews = (name: string) =>
    setOpenViews((prev) => ({ ...prev, [name]: !prev[name] }));
  const toggleRoutines = (name: string) =>
    setOpenRoutines((prev) => ({ ...prev, [name]: !prev[name] }));

  /** Resolve the table list the right-click menu should target. If
   *  the right-clicked node is part of the active multi-selection,
   *  the menu acts on the whole batch; otherwise it acts on the
   *  single node (and the multi-selection is left intact in case the
   *  user only wanted to peek). */
  const resolveTargetTables = (clickedNode: DbSchemaNode, dbName: string): string[] => {
    if (multiSel.has(clickedNode.id) && multiSel.size > 1) {
      // Map ids → labels. Keep order stable by walking the
      // database's table list in display order.
      const db = databases.find((d) => d.name === dbName);
      if (!db) return [clickedNode.label];
      return db.tables.filter((n) => multiSel.has(n.id)).map((n) => n.label);
    }
    return [clickedNode.label];
  };

  function buildRootMenu(): ContextMenuItem[] {
    const items: ContextMenuItem[] = [];
    if (actions?.onCreateDatabase) {
      items.push({ label: t("New database…"), action: () => actions.onCreateDatabase!() });
    }
    if (actions?.onImportSql) {
      items.push({ label: t("Import SQL…"), action: () => actions.onImportSql!() });
    }
    return items;
  }

  function buildDatabaseMenu(dbName: string, isCurrent: boolean): ContextMenuItem[] {
    const items: ContextMenuItem[] = [];
    if (actions?.onRefreshDatabase) {
      items.push({
        label: t("Refresh"),
        action: () => actions.onRefreshDatabase!(dbName),
        disabled: !isCurrent,
      });
    }
    if (actions?.onExportDatabase) {
      items.push({
        label: t("Export database…"),
        action: () => actions.onExportDatabase!(dbName),
      });
    }
    if (actions?.onCreateDatabase || actions?.onImportSql) {
      if (items.length > 0) items.push({ divider: true });
      if (actions?.onCreateDatabase) {
        items.push({ label: t("New database…"), action: () => actions.onCreateDatabase!() });
      }
      if (actions?.onImportSql) {
        items.push({ label: t("Import SQL…"), action: () => actions.onImportSql!() });
      }
    }
    if (actions?.onDropDatabase) {
      if (items.length > 0) items.push({ divider: true });
      items.push({
        label: t("Drop database…"),
        action: () => actions.onDropDatabase!(dbName),
      });
    }
    return items;
  }

  function buildTableMenu(dbName: string, tables: string[]): ContextMenuItem[] {
    const items: ContextMenuItem[] = [];
    const multi = tables.length > 1;
    if (multi) {
      items.push({ section: t("{n} tables selected", { n: tables.length }) });
    }
    if (actions?.onCopyTableName) {
      items.push({
        label: multi ? t("Copy {n} names", { n: tables.length }) : t("Copy name"),
        action: () => actions.onCopyTableName!(dbName, tables),
      });
    }
    if (actions?.onExportTables) {
      items.push({
        label: multi
          ? t("Export {n} tables…", { n: tables.length })
          : t("Export table…"),
        action: () => actions.onExportTables!(dbName, tables),
      });
    }
    if (actions?.onTruncateTables || actions?.onDropTables) {
      if (items.length > 0) items.push({ divider: true });
      if (actions?.onTruncateTables) {
        items.push({
          label: multi ? t("Truncate {n} tables…", { n: tables.length }) : t("Truncate…"),
          action: () => actions.onTruncateTables!(dbName, tables),
        });
      }
      if (actions?.onDropTables) {
        items.push({
          label: multi ? t("Drop {n} tables…", { n: tables.length }) : t("Drop table…"),
          action: () => actions.onDropTables!(dbName, tables),
        });
      }
    }
    return items;
  }

  function openMenu(e: ReactMouseEvent, items: ContextMenuItem[]) {
    if (items.length === 0) return; // no actions wired → no menu
    e.preventDefault();
    e.stopPropagation();
    setMenu({ x: e.clientX, y: e.clientY, items });
  }

  function handleTableClick(
    e: ReactMouseEvent,
    db: DbSchemaDatabase,
    node: DbSchemaNode,
    indexInDb: number,
  ) {
    // cmd / ctrl → toggle in multi-selection, don't change primary
    if (e.metaKey || e.ctrlKey) {
      e.preventDefault();
      setMultiSel((prev) => {
        const next = new Set(prev);
        if (next.has(node.id)) next.delete(node.id);
        else next.add(node.id);
        return next;
      });
      anchorRef.current = { db: db.name, index: indexInDb };
      return;
    }
    // shift → range select against the anchor (or last clicked)
    if (e.shiftKey && anchorRef.current && anchorRef.current.db === db.name) {
      e.preventDefault();
      const [a, b] = [anchorRef.current.index, indexInDb].sort((x, y) => x - y);
      const range = db.tables.slice(a, b + 1).map((n) => n.id);
      setMultiSel(new Set(range));
      return;
    }
    // Plain click → reset multi-selection and let the panel handle
    // the primary selection.
    setMultiSel(new Set([node.id]));
    anchorRef.current = { db: db.name, index: indexInDb };
    onSelectTable(db.name, node);
  }

  return (
    <div
      className="dbt"
      onContextMenu={(e) => {
        // Tree-area background right-click → root menu.
        if (e.target === e.currentTarget) openMenu(e, buildRootMenu());
      }}
    >
      <div className="dbt-search">
        <Search size={11} />
        <input
          placeholder={t("Filter tables…")}
          value={q}
          onChange={(e) => setQ(e.currentTarget.value)}
        />
        {q && (
          <button
            type="button"
            className="mini-button mini-button--ghost"
            onClick={() => setQ("")}
            title={t("Clear")}
          >
            <X size={10} />
          </button>
        )}
      </div>
      <div
        className="dbt-body"
        onContextMenu={(e) => {
          // Empty area inside the body → root menu too.
          if (e.target === e.currentTarget) openMenu(e, buildRootMenu());
        }}
      >
        {visible.length === 0 && (
          <div className="dbt-empty">{t("No databases.")}</div>
        )}
        {visible.map((db) => {
          const dbOpen = openDb[db.name] ?? db.current;
          const tablesOpen = openTables[db.name] ?? db.current;
          return (
            <div key={db.name}>
              <button
                type="button"
                className={"dbt-row lvl-0" + (db.current ? " current" : "")}
                onClick={() => {
                  toggleDb(db.name);
                  if (!db.current) onSelectDatabase?.(db.name);
                }}
                onContextMenu={(e) => openMenu(e, buildDatabaseMenu(db.name, db.current))}
              >
                <span className="dbt-caret">
                  {dbOpen ? <ChevronDown size={10} /> : <ChevronRight size={10} />}
                </span>
                <Database size={11} style={{ color: "var(--accent)" }} />
                <span className="dbt-name">{db.name}</span>
                {db.current && <span className="dbt-pill">{t("current")}</span>}
              </button>

              {dbOpen && db.current && db.schemas && db.schemas.length > 0 && (
                <>
                  <div className="dbt-row lvl-1 group" aria-disabled>
                    <span className="dbt-caret" />
                    <FolderTree size={10} />
                    <span className="dbt-name">{t("Schemas")}</span>
                    <span className="dbt-count">{db.schemas.length}</span>
                  </div>
                  {db.schemas.map((schema) => {
                    const active = schema === (db.activeSchema || "");
                    return (
                      <button
                        key={schema}
                        type="button"
                        className={"dbt-row lvl-2 leaf" + (active ? " sel" : "")}
                        onClick={() => {
                          if (!active) onSelectSchema?.(db.name, schema);
                        }}
                      >
                        <span className="dbt-caret" />
                        <FolderTree size={10} style={{ color: "var(--muted)" }} />
                        <span className="dbt-name">{schema}</span>
                        {active && <span className="dbt-pill">{t("current")}</span>}
                      </button>
                    );
                  })}
                </>
              )}
              {dbOpen && db.current && (
                <>
                  <button
                    type="button"
                    className="dbt-row lvl-1 group"
                    onClick={() => toggleTables(db.name)}
                  >
                    <span className="dbt-caret">
                      {tablesOpen ? <ChevronDown size={10} /> : <ChevronRight size={10} />}
                    </span>
                    <Table size={10} />
                    <span className="dbt-name">{t("Tables")}</span>
                    <span className="dbt-count">{db.tables.length}</span>
                  </button>
                  {tablesOpen && db.tables.length === 0 && (
                    <div className="dbt-empty">{t("No tables in this database.")}</div>
                  )}
                  {tablesOpen &&
                    db.tables.map((node, idx) => {
                      const isSelected = node.id === selectedTableId;
                      const isMulti = multiSel.has(node.id) && multiSel.size > 1;
                      return (
                        <button
                          key={node.id}
                          type="button"
                          className={
                            "dbt-row lvl-2 leaf" +
                            (isSelected ? " sel" : "") +
                            (isMulti ? " is-multi-selected" : "")
                          }
                          onClick={(e) => handleTableClick(e, db, node, idx)}
                          onContextMenu={(e) => {
                            const targets = resolveTargetTables(node, db.name);
                            openMenu(e, buildTableMenu(db.name, targets));
                          }}
                          title={node.tooltip ?? undefined}
                        >
                          <span className="dbt-caret" />
                          <Table size={10} style={{ color: "var(--muted)" }} />
                          <span className="dbt-name">{node.label}</span>
                          {typeof node.count === "number" && (
                            <span className="dbt-count">{node.count.toLocaleString()}</span>
                          )}
                        </button>
                      );
                    })}

                  {db.views && db.views.length > 0 && (
                    <>
                      <button
                        type="button"
                        className="dbt-row lvl-1 group"
                        onClick={() => toggleViews(db.name)}
                      >
                        <span className="dbt-caret">
                          {(openViews[db.name] ?? false) ? (
                            <ChevronDown size={10} />
                          ) : (
                            <ChevronRight size={10} />
                          )}
                        </span>
                        <Eye size={10} />
                        <span className="dbt-name">{t("Views")}</span>
                        <span className="dbt-count">{db.views.length}</span>
                      </button>
                      {(openViews[db.name] ?? false) &&
                        db.views.map((node) => {
                          const isSelected = node.id === selectedTableId;
                          return (
                            <button
                              key={node.id}
                              type="button"
                              className={"dbt-row lvl-2 leaf" + (isSelected ? " sel" : "")}
                              onClick={() => onSelectTable(db.name, node)}
                              title={node.tooltip ?? undefined}
                            >
                              <span className="dbt-caret" />
                              <Eye size={10} style={{ color: "var(--muted)" }} />
                              <span className="dbt-name">{node.label}</span>
                            </button>
                          );
                        })}
                    </>
                  )}

                  {db.routines && db.routines.length > 0 && (
                    <>
                      <button
                        type="button"
                        className="dbt-row lvl-1 group"
                        onClick={() => toggleRoutines(db.name)}
                      >
                        <span className="dbt-caret">
                          {(openRoutines[db.name] ?? false) ? (
                            <ChevronDown size={10} />
                          ) : (
                            <ChevronRight size={10} />
                          )}
                        </span>
                        <FunctionSquare size={10} />
                        <span className="dbt-name">{t("Routines")}</span>
                        <span className="dbt-count">{db.routines.length}</span>
                      </button>
                      {(openRoutines[db.name] ?? false) &&
                        db.routines.map((node) => {
                          // Routines don't have a per-row "select"
                          // semantics in M-1 — we render them so
                          // the user knows what's there but the
                          // click is a no-op for now (a future PR
                          // can add a definition viewer).
                          return (
                            <div
                              key={node.id}
                              className="dbt-row lvl-2 leaf is-readonly"
                              title={node.tooltip ?? undefined}
                            >
                              <span className="dbt-caret" />
                              <FunctionSquare
                                size={10}
                                style={{ color: "var(--muted)" }}
                              />
                              <span className="dbt-name">{node.label}</span>
                              {node.badge ? (
                                <span className="dbt-count">{node.badge}</span>
                              ) : null}
                            </div>
                          );
                        })}
                    </>
                  )}
                </>
              )}
            </div>
          );
        })}
      </div>
      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          items={menu.items}
          onClose={() => setMenu(null)}
        />
      )}
    </div>
  );
}
