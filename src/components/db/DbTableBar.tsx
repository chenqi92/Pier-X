import { ChevronDown, Database, Search, Table } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";

import { useI18n } from "../../i18n/useI18n";
import type { DbSchemaDatabase, DbSchemaNode } from "./DbSchemaTree";

type Props = {
  databases: DbSchemaDatabase[];
  selectedTableId: string | null;
  onSelectTable: (databaseName: string, node: DbSchemaNode) => void;
  onSelectDatabase?: (name: string) => void;
};

/**
 * Compact horizontal switcher used at narrow panel widths in place of
 * the left-rail schema tree. Renders the current database (with a
 * dropdown when more are available) followed by a single dropdown
 * picker for the active table — flat chips don't scale past a handful
 * of tables on a narrow sidebar, so we always present the table list
 * as a searchable popover.
 */
export default function DbTableBar({
  databases,
  selectedTableId,
  onSelectTable,
  onSelectDatabase,
}: Props) {
  const { t } = useI18n();
  const current = databases.find((d) => d.current) ?? databases[0];
  const others = databases.filter((d) => d !== current);
  const [dbOpen, setDbOpen] = useState(false);
  const [tableOpen, setTableOpen] = useState(false);
  const [filter, setFilter] = useState("");
  const dbWrapRef = useRef<HTMLDivElement | null>(null);
  const tableWrapRef = useRef<HTMLDivElement | null>(null);
  const filterInputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    if (!dbOpen) return;
    const onDocClick = (e: MouseEvent) => {
      if (!dbWrapRef.current?.contains(e.target as Node)) setDbOpen(false);
    };
    document.addEventListener("mousedown", onDocClick);
    return () => document.removeEventListener("mousedown", onDocClick);
  }, [dbOpen]);

  useEffect(() => {
    if (!tableOpen) return;
    const onDocClick = (e: MouseEvent) => {
      if (!tableWrapRef.current?.contains(e.target as Node)) {
        setTableOpen(false);
        setFilter("");
      }
    };
    document.addEventListener("mousedown", onDocClick);
    return () => document.removeEventListener("mousedown", onDocClick);
  }, [tableOpen]);

  // Auto-focus the filter input on open so the user can start typing
  // immediately. Reset the filter when closing so reopening doesn't
  // surprise them with a stale narrowing.
  useEffect(() => {
    if (tableOpen) {
      requestAnimationFrame(() => filterInputRef.current?.focus());
    }
  }, [tableOpen]);

  const tables = current?.tables ?? [];
  const selectedTable = useMemo(
    () => tables.find((node) => node.id === selectedTableId) ?? null,
    [tables, selectedTableId],
  );
  const filteredTables = useMemo(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return tables;
    return tables.filter((node) => node.label.toLowerCase().includes(q));
  }, [tables, filter]);

  if (!current) {
    return (
      <div className="db2-tablebar db2-tablebar--empty">
        <span className="db2-tablebar__hint">{t("No databases.")}</span>
      </div>
    );
  }

  const canPickDb = others.length > 0 && !!onSelectDatabase;
  const tableButtonLabel = selectedTable?.label ?? t("Pick a table…");

  return (
    <div className="db2-tablebar">
      <div className="db2-tablebar__db" ref={dbWrapRef}>
        <button
          type="button"
          className={"db2-tablebar__db-btn" + (canPickDb ? " is-pickable" : "")}
          disabled={!canPickDb}
          onClick={() => canPickDb && setDbOpen((v) => !v)}
          title={current.name}
        >
          <Database size={11} />
          <span className="db2-tablebar__db-name">{current.name}</span>
          {canPickDb && <ChevronDown size={10} />}
        </button>
        {dbOpen && (
          <div className="db2-tablebar__db-menu" role="menu">
            {others.map((db) => (
              <button
                key={db.name}
                type="button"
                role="menuitem"
                className="db2-tablebar__db-item"
                onClick={() => {
                  setDbOpen(false);
                  onSelectDatabase?.(db.name);
                }}
              >
                <Database size={10} />
                <span>{db.name}</span>
              </button>
            ))}
          </div>
        )}
      </div>
      <div className="db2-tablebar__sep" aria-hidden />
      <div
        className="db2-tablebar__db db2-tablebar__tables"
        ref={tableWrapRef}
      >
        <button
          type="button"
          className={
            "db2-tablebar__db-btn is-pickable db2-tablebar__table-btn" +
            (selectedTable ? "" : " is-empty")
          }
          disabled={tables.length === 0}
          onClick={() => setTableOpen((v) => !v)}
          title={
            selectedTable
              ? selectedTable.label
              : tables.length === 0
                ? t("No tables in this database.")
                : t("Pick a table…")
          }
        >
          <Table size={11} />
          <span className="db2-tablebar__db-name">{tableButtonLabel}</span>
          <span className="db2-tablebar__table-count">{tables.length}</span>
          <ChevronDown size={10} />
        </button>
        {tableOpen && (
          <div className="db2-tablebar__db-menu db2-tablebar__table-menu" role="menu">
            <div className="db2-tablebar__filter">
              <Search size={10} />
              <input
                ref={filterInputRef}
                value={filter}
                onChange={(e) => setFilter(e.target.value)}
                placeholder={t("Filter tables…")}
                spellCheck={false}
              />
            </div>
            {filteredTables.length === 0 ? (
              <div className="db2-tablebar__empty">
                {tables.length === 0
                  ? t("No tables in this database.")
                  : t("No matching tables.")}
              </div>
            ) : (
              filteredTables.map((node) => {
                const selected = node.id === selectedTableId;
                return (
                  <button
                    key={node.id}
                    type="button"
                    role="menuitem"
                    className={
                      "db2-tablebar__db-item" + (selected ? " is-selected" : "")
                    }
                    onClick={() => {
                      setTableOpen(false);
                      setFilter("");
                      onSelectTable(current.name, node);
                    }}
                  >
                    <Table size={10} />
                    <span>{node.label}</span>
                  </button>
                );
              })
            )}
          </div>
        )}
      </div>
    </div>
  );
}
