import type { QueryExecutionResult } from "../lib/types";
import { useI18n } from "../i18n/useI18n";

type Props = {
  result: QueryExecutionResult | null;
  error: string;
  emptyLabel: string;
};

export default function QueryResultPanel({ result, error, emptyLabel }: Props) {
  const { t } = useI18n();

  if (error) {
    return <div className="status-note status-note--error">{error}</div>;
  }
  if (!result) {
    return <div className="empty-note">{emptyLabel}</div>;
  }

  return (
    <div className="data-table-wrap ux-selectable">
      <table className="data-table">
        <thead>
          <tr>
            {result.columns.map((col) => (
              <th key={col}>{col}</th>
            ))}
          </tr>
        </thead>
        <tbody>
          {result.rows.map((row, i) => (
            <tr key={i}>
              {row.map((cell, j) => (
                <td key={j}>{cell}</td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
      <div className="inline-note">
        {t("{rows} affected · {elapsed} ms", {
          rows: result.affectedRows,
          elapsed: result.elapsedMs,
        })}
        {result.truncated ? t(" · truncated") : ""}
      </div>
    </div>
  );
}
