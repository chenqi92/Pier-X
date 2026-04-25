import { ChevronDown, Database, Table } from "lucide-react";
import { useEffect, useRef, useState } from "react";

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
 * dropdown when more are available) followed by a horizontally
 * scrollable strip of table chips.
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
  const wrapRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!dbOpen) return;
    const onDocClick = (e: MouseEvent) => {
      if (!wrapRef.current?.contains(e.target as Node)) setDbOpen(false);
    };
    document.addEventListener("mousedown", onDocClick);
    return () => document.removeEventListener("mousedown", onDocClick);
  }, [dbOpen]);

  if (!current) {
    return (
      <div className="db2-tablebar db2-tablebar--empty">
        <span className="db2-tablebar__hint">{t("No databases.")}</span>
      </div>
    );
  }

  const canPickDb = others.length > 0 && !!onSelectDatabase;

  return (
    <div className="db2-tablebar">
      <div className="db2-tablebar__db" ref={wrapRef}>
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
      <div className="db2-tablebar__chips" role="tablist">
        {current.tables.length === 0 ? (
          <span className="db2-tablebar__hint">{t("No tables in this database.")}</span>
        ) : (
          current.tables.map((node) => {
            const selected = node.id === selectedTableId;
            return (
              <button
                key={node.id}
                type="button"
                role="tab"
                aria-selected={selected}
                className={"db2-tablebar__chip" + (selected ? " is-selected" : "")}
                onClick={() => onSelectTable(current.name, node)}
                title={node.label}
              >
                <Table size={10} />
                <span className="db2-tablebar__chip-label">{node.label}</span>
              </button>
            );
          })
        )}
      </div>
    </div>
  );
}
