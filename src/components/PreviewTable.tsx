import { useI18n } from "../i18n/useI18n";
import type { DataPreview } from "../lib/types";

type Props = {
  preview: DataPreview | null;
  emptyLabel: string;
};

export default function PreviewTable({ preview, emptyLabel }: Props) {
  const { t } = useI18n();

  if (!preview) {
    return <div className="empty-note">{emptyLabel}</div>;
  }

  return (
    <div className="data-table-wrap ux-selectable">
      <table className="data-table">
        <thead>
          <tr>
            {preview.columns.map((col) => (
              <th key={col}>{col}</th>
            ))}
          </tr>
        </thead>
        <tbody>
          {preview.rows.map((row, i) => (
            <tr key={i}>
              {row.map((cell, j) => (
                <td key={j}>{cell}</td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
      {preview.truncated ? (
        <div className="inline-note">{t("Results truncated.")}</div>
      ) : null}
    </div>
  );
}
