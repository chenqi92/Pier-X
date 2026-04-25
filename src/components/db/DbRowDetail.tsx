import { ArrowRight, Copy, X } from "lucide-react";

import { useI18n } from "../../i18n/useI18n";
import { writeClipboardText } from "../../lib/clipboard";

type Column = { name: string; type?: string | null; pk?: boolean };

/** A foreign-key edge displayed in the detail drawer. */
export type DbRowFk = {
  /** Display label, e.g. "shipment_items (38)". */
  label: string;
  /** Optional click handler — when omitted the link is non-interactive. */
  onClick?: () => void;
};

type Props = {
  title: string;
  columns: Column[];
  row: string[];
  onClose: () => void;
  /** Outbound FK edges to render under the field list. */
  foreignKeys?: DbRowFk[];
};

/**
 * Right-drawer row detail. Read-only for the pilot — inline edit and
 * FK navigation are design-only placeholders (docs/BACKEND-GAPS.md).
 */
export default function DbRowDetail({ title, columns, row, onClose, foreignKeys }: Props) {
  const { t } = useI18n();

  const copyJson = () => {
    const pairs: Record<string, string> = {};
    columns.forEach((col, i) => {
      pairs[col.name] = row[i] ?? "";
    });
    void writeClipboardText(JSON.stringify(pairs, null, 2));
  };

  return (
    <aside className="rg-detail" aria-label={t("Row detail")}>
      <div className="rg-detail-head">
        <span className="rg-detail-title">{title}</span>
        <span className="rg-detail-spacer" />
        <button
          type="button"
          className="mini-button mini-button--ghost"
          onClick={copyJson}
          title={t("Copy as JSON")}
        >
          <Copy size={11} />
        </button>
        <button
          type="button"
          className="mini-button mini-button--ghost"
          onClick={onClose}
          title={t("Close")}
        >
          <X size={11} />
        </button>
      </div>
      <div className="rg-detail-body">
        {columns.map((col, i) => {
          const value = row[i];
          const isNull = value === null || value === undefined || value === "";
          return (
            <div key={col.name} className="rg-detail-field">
              <div className="rg-detail-label">
                {col.pk && <span className="rg-pk">{t("PK")}</span>}
                {col.name}
                {col.type && <span className="rg-detail-type">{col.type}</span>}
              </div>
              <div className="rg-detail-value">
                {isNull ? <span className="rg-null">NULL</span> : value}
              </div>
            </div>
          );
        })}
        {foreignKeys && foreignKeys.length > 0 && (
          <div className="rg-detail-field rg-detail-fks">
            <div className="rg-detail-label">{t("Foreign keys")}</div>
            <div className="rg-detail-fk-list">
              {foreignKeys.map((fk, i) => (
                <button
                  key={i}
                  type="button"
                  className="rg-fk"
                  onClick={fk.onClick}
                  disabled={!fk.onClick}
                >
                  {fk.label}
                  <ArrowRight size={9} />
                </button>
              ))}
            </div>
          </div>
        )}
      </div>
    </aside>
  );
}
