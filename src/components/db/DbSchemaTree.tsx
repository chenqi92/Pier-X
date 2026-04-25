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
import { useMemo, useState } from "react";

import { useI18n } from "../../i18n/useI18n";

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

type Props = {
  databases: DbSchemaDatabase[];
  selectedTableId: string | null;
  onSelectTable: (databaseName: string, node: DbSchemaNode) => void;
  onSelectDatabase?: (name: string) => void;
  /** Fired when the user clicks a non-active schema row. The
   *  panel re-runs `postgresBrowse` with the new schema. */
  onSelectSchema?: (databaseName: string, schema: string) => void;
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

  return (
    <div className="dbt">
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
      <div className="dbt-body">
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
                    db.tables.map((node) => {
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
    </div>
  );
}
