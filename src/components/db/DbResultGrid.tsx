import type { ReactNode } from "react";

import { useI18n } from "../../i18n/useI18n";
import type { DataPreview } from "../../lib/types";

type Props = {
  preview: DataPreview | null;
  /** Primary-key column names (rendered with a PK badge and right-aligned). */
  pkColumns?: string[];
  /** Right-aligned numeric columns (style hint). */
  numericColumns?: string[];
  toolbar?: ReactNode;
  /** Enables per-row click → opens the row detail drawer. */
  onOpenRow?: (row: string[]) => void;
  emptyLabel?: string;
};

/**
 * Sticky-header, mono-font, token-coloured result grid. For the pilot
 * port this reads directly from the panel's `DataPreview` snapshot —
 * server paging and inline CRUD are tracked in docs/BACKEND-GAPS.md.
 */
export default function DbResultGrid({
  preview,
  pkColumns,
  numericColumns,
  toolbar,
  onOpenRow,
  emptyLabel,
}: Props) {
  const { t } = useI18n();

  if (!preview) {
    return (
      <div className="rg">
        {toolbar && <div className="rg-toolbar">{toolbar}</div>}
        <div className="rg-empty">{emptyLabel ?? t("No rows to show.")}</div>
      </div>
    );
  }

  const pkSet = new Set(pkColumns ?? []);
  const numericSet = new Set(numericColumns ?? []);

  return (
    <div className="rg">
      <div className="rg-toolbar">
        <span className="rg-stat">
          <b>{preview.rows.length.toLocaleString()}</b>
          <span className="rg-stat-muted"> {t("rows")}</span>
          {preview.truncated && (
            <span className="rg-stat-muted"> · {t("truncated")}</span>
          )}
        </span>
        {toolbar}
      </div>

      <div className="rg-scroll">
        <table className="rg-table">
          <thead>
            <tr>
              <th className="rg-th-n">#</th>
              {preview.columns.map((col) => {
                const isPk = pkSet.has(col);
                const align = numericSet.has(col) ? "right" : "left";
                return (
                  <th key={col} style={{ textAlign: align }}>
                    <div className="rg-th-body">
                      {isPk && <span className="rg-pk">{t("PK")}</span>}
                      <span className="rg-th-name">{col}</span>
                    </div>
                  </th>
                );
              })}
            </tr>
          </thead>
          <tbody>
            {preview.rows.length === 0 ? (
              <tr>
                <td
                  className="rg-empty"
                  colSpan={preview.columns.length + 1}
                  style={{ textAlign: "center" }}
                >
                  {emptyLabel ?? t("No rows to show.")}
                </td>
              </tr>
            ) : (
              preview.rows.map((row, i) => (
                <tr
                  key={i}
                  className="rg-row"
                  onClick={() => onOpenRow?.(row)}
                  style={{ cursor: onOpenRow ? "pointer" : undefined }}
                >
                  <td className="rg-td-n">{i + 1}</td>
                  {row.map((cell, ci) => {
                    const col = preview.columns[ci];
                    const isPk = pkSet.has(col);
                    const isNum = numericSet.has(col);
                    const isNull = cell === null || cell === "" || cell === "NULL";
                    const className =
                      "rg-td" +
                      (isNum ? " rg-td-num" : "") +
                      (isPk ? " rg-td-pk" : "");
                    return (
                      <td
                        key={ci}
                        className={className}
                        style={{ textAlign: isNum ? "right" : "left" }}
                      >
                        {isNull ? <span className="rg-null">NULL</span> : cell}
                      </td>
                    );
                  })}
                </tr>
              ))
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}
