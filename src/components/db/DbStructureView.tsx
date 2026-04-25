import { KeyRound } from "lucide-react";
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

type Props = {
  columns: DbStructureColumn[];
  /** Color the type cells with the engine accent (e.g. `var(--svc-mysql)`). */
  typeAccentVar: string;
  /** Optional footer note explaining what's not yet shown (indexes / FKs). */
  footnote?: ReactNode;
};

export default function DbStructureView({ columns, typeAccentVar, footnote }: Props) {
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
      {footnote && <div className="db2-structure__footnote">{footnote}</div>}
    </div>
  );
}
