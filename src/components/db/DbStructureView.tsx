import { ArrowRight, KeyRound, Link2, Zap } from "lucide-react";
import type { CSSProperties, ReactNode } from "react";

import { useI18n } from "../../i18n/useI18n";
import DbStubView from "./DbStubView";

export type DbStructureColumn = {
  name: string;
  type: string;
  /** Whether the column is part of the primary key. */
  pk: boolean;
  /** Whether the column allows NULL. */
  nullable: boolean;
  /** Engine-specific extra column for the "Key" position (e.g. MySQL's
   *  `MUL`/`UNI` index hint). Free-form text — empty / undefined renders as a dash. */
  keyHint?: string;
  /** Default value, if any. Empty/undefined renders as a dash. */
  defaultValue?: string | null;
  /** Free-form extra (MySQL `EXTRA`, PG attoptions). Optional. */
  extra?: string;
};

/** Index row rendered in the Structure tab's Indexes section.
 *  `unique` controls the badge tint; `kind` shows up as the
 *  small uppercase suffix (BTREE / HASH / GIN / …). */
export type DbStructureIndex = {
  name: string;
  columns: string[];
  unique: boolean;
  kind: string;
};

/** Trigger row rendered in the Structure tab's Triggers
 *  section. SQLite-only for now — `event` is e.g. "BEFORE INSERT"
 *  or "AFTER UPDATE", and `sql` is the full CREATE TRIGGER
 *  statement (shown on hover via `title`). */
export type DbStructureTrigger = {
  name: string;
  event: string;
  sql: string;
};

/** Foreign-key row rendered in the Structure tab's Foreign keys
 *  section. Composite keys pair `columns` with `refColumns` by
 *  index — the renderer joins them with " · " for display. */
export type DbStructureForeignKey = {
  name: string;
  columns: string[];
  refSchema: string;
  refTable: string;
  refColumns: string[];
  onUpdate: string;
  onDelete: string;
};

type Props = {
  columns: DbStructureColumn[];
  /** Color the type cells with the engine accent (e.g. `var(--svc-mysql)`). */
  typeAccentVar: string;
  /** Optional Indexes section. Empty / undefined hides the
   *  section entirely so older callers (or tables with no
   *  indexes) don't show an empty header. */
  indexes?: DbStructureIndex[];
  /** Optional Foreign keys section. Same omit-when-empty
   *  semantics as `indexes`. */
  foreignKeys?: DbStructureForeignKey[];
  /** Optional Triggers section (SQLite). Same omit-when-empty
   *  semantics as `indexes`. */
  triggers?: DbStructureTrigger[];
  /** Optional footer note. Use it for "this is what we still
   *  don't show" callouts; the indexes / FK gaps are now
   *  closed so the note typically goes blank. */
  footnote?: ReactNode;
};

export default function DbStructureView({
  columns,
  typeAccentVar,
  indexes,
  foreignKeys,
  triggers,
  footnote,
}: Props) {
  const { t } = useI18n();

  if (columns.length === 0) {
    return <DbStubView title={t("No table selected")} />;
  }

  const typeStyle: CSSProperties = { color: typeAccentVar };

  return (
    <div className="db2-structure">
      <div className="db2-structure__head">
        <span className="db2-structure__title">{t("Columns")}</span>
        <span className="db2-structure__count">{columns.length}</span>
      </div>
      <div className="db2-structure__scroll">
        <table className="rg-table db2-structure__table">
          <thead>
            <tr>
              <th className="db2-structure__th-pk" />
              <th>
                <div className="rg-th-body">
                  <span className="rg-th-name">{t("Name")}</span>
                </div>
              </th>
              <th>
                <div className="rg-th-body">
                  <span className="rg-th-name">{t("Type")}</span>
                </div>
              </th>
              <th>
                <div className="rg-th-body">
                  <span className="rg-th-name">{t("Null")}</span>
                </div>
              </th>
              <th>
                <div className="rg-th-body">
                  <span className="rg-th-name">{t("Default")}</span>
                </div>
              </th>
              <th>
                <div className="rg-th-body">
                  <span className="rg-th-name">{t("Extra")}</span>
                </div>
              </th>
            </tr>
          </thead>
          <tbody>
            {columns.map((col) => {
              const def = col.defaultValue;
              const defText =
                def === null || def === undefined || def === "" ? "—" : def;
              const extraBits = [col.keyHint, col.extra].filter((s): s is string => !!s && s.trim() !== "");
              return (
                <tr key={col.name} className="rg-row">
                  <td className="rg-td db2-structure__td-pk">
                    {col.pk && (
                      <span
                        className="rg-pk"
                        title={t("Primary key")}
                        aria-label={t("Primary key")}
                      >
                        <KeyRound size={11} />
                      </span>
                    )}
                  </td>
                  <td className="rg-td">{col.name}</td>
                  <td className="rg-td" style={typeStyle}>
                    {col.type}
                  </td>
                  <td className="rg-td">{col.nullable ? t("YES") : t("NO")}</td>
                  <td className="rg-td">
                    {def === null ? <span className="rg-null">NULL</span> : defText}
                  </td>
                  <td className="rg-td">
                    {extraBits.length > 0 ? extraBits.join(" · ") : "—"}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      {indexes && indexes.length > 0 && (
        <div className="db2-structure__section">
          <div className="db2-structure__head">
            <span className="db2-structure__title">{t("Indexes")}</span>
            <span className="db2-structure__count">{indexes.length}</span>
          </div>
          <div className="db2-structure__list">
            {indexes.map((idx) => (
              <div key={idx.name} className="db2-structure__row">
                <span className="db2-structure__row-icon" aria-hidden>
                  <KeyRound size={11} />
                </span>
                <span className="db2-structure__row-name">{idx.name}</span>
                <span className="db2-structure__row-cols">
                  {idx.columns.join(", ")}
                </span>
                <span className="db2-structure__row-meta">
                  {idx.unique ? t("UNIQUE") : t("INDEX")}
                  {idx.kind ? ` · ${idx.kind.toUpperCase()}` : ""}
                </span>
              </div>
            ))}
          </div>
        </div>
      )}

      {foreignKeys && foreignKeys.length > 0 && (
        <div className="db2-structure__section">
          <div className="db2-structure__head">
            <span className="db2-structure__title">{t("Foreign keys")}</span>
            <span className="db2-structure__count">{foreignKeys.length}</span>
          </div>
          <div className="db2-structure__list">
            {foreignKeys.map((fk) => {
              const target = fk.refSchema
                ? `${fk.refSchema}.${fk.refTable}`
                : fk.refTable;
              const cascade =
                fk.onUpdate && fk.onDelete
                  ? t("ON UPDATE {u} · ON DELETE {d}", {
                      u: fk.onUpdate,
                      d: fk.onDelete,
                    })
                  : "";
              return (
                <div key={fk.name} className="db2-structure__row">
                  <span className="db2-structure__row-icon" aria-hidden>
                    <Link2 size={11} />
                  </span>
                  <span className="db2-structure__row-name">{fk.name}</span>
                  <span className="db2-structure__row-cols">
                    {fk.columns.join(", ")}
                    <ArrowRight size={9} className="db2-structure__row-arrow" />
                    <span className="db2-structure__row-target">{target}</span>
                    {fk.refColumns.length > 0 && (
                      <>
                        {" · "}
                        {fk.refColumns.join(", ")}
                      </>
                    )}
                  </span>
                  {cascade && (
                    <span className="db2-structure__row-meta">{cascade}</span>
                  )}
                </div>
              );
            })}
          </div>
        </div>
      )}

      {triggers && triggers.length > 0 && (
        <div className="db2-structure__section">
          <div className="db2-structure__head">
            <span className="db2-structure__title">{t("Triggers")}</span>
            <span className="db2-structure__count">{triggers.length}</span>
          </div>
          <div className="db2-structure__list">
            {triggers.map((tr) => (
              <div
                key={tr.name}
                className="db2-structure__row"
                title={tr.sql}
              >
                <span className="db2-structure__row-icon" aria-hidden>
                  <Zap size={11} />
                </span>
                <span className="db2-structure__row-name">{tr.name}</span>
                <span className="db2-structure__row-cols">{tr.event}</span>
                <span className="db2-structure__row-meta" />
              </div>
            ))}
          </div>
        </div>
      )}

      {footnote && <div className="db2-structure__footnote">{footnote}</div>}
    </div>
  );
}
