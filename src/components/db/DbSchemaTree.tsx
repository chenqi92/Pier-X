import { ChevronDown, ChevronRight, Database, Search, Table, X } from "lucide-react";
import { useMemo, useState } from "react";

import { useI18n } from "../../i18n/useI18n";

export type DbSchemaNode = {
  id: string;
  label: string;
  count?: number | null;
};

export type DbSchemaDatabase = {
  name: string;
  current: boolean;
  tables: DbSchemaNode[];
};

type Props = {
  databases: DbSchemaDatabase[];
  selectedTableId: string | null;
  onSelectTable: (databaseName: string, node: DbSchemaNode) => void;
  onSelectDatabase?: (name: string) => void;
};

/**
 * Left-rail schema tree. The current backend only exposes
 * `databases[]` + `tables[]` for the active DB, so this tree is
 * intentionally shallow (no views / routines / functions yet — see
 * docs/BACKEND-GAPS.md). Non-current databases collapse to a stub
 * row the user can click to switch.
 */
export default function DbSchemaTree({
  databases,
  selectedTableId,
  onSelectTable,
  onSelectDatabase,
}: Props) {
  const { t } = useI18n();
  const [openDb, setOpenDb] = useState<Record<string, boolean>>(() =>
    Object.fromEntries(databases.filter((d) => d.current).map((d) => [d.name, true])),
  );
  const [openTables, setOpenTables] = useState<Record<string, boolean>>(() =>
    Object.fromEntries(databases.filter((d) => d.current).map((d) => [d.name, true])),
  );
  const [q, setQ] = useState("");

  const qLower = q.trim().toLowerCase();
  const visible = useMemo(() => {
    if (!qLower) return databases;
    return databases.map((db) => ({
      ...db,
      tables: db.tables.filter((t) => t.label.toLowerCase().includes(qLower)),
    }));
  }, [databases, qLower]);

  const toggleDb = (name: string) =>
    setOpenDb((prev) => ({ ...prev, [name]: !prev[name] }));
  const toggleTables = (name: string) =>
    setOpenTables((prev) => ({ ...prev, [name]: !prev[name] }));

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
                </>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
