import { FileText, Lock, Play, Unlock } from "lucide-react";
import { useMemo } from "react";

import { useI18n } from "../../i18n/useI18n";
import { renderSqlTokens } from "./sqlHighlight";

type Props = {
  /** Current tab/query name. Multi-tab support is planned — see BACKEND-GAPS.md. */
  tabName?: string;
  sql: string;
  onChange: (next: string) => void;
  writable: boolean;
  onToggleWrite: () => void;
  /** When a write-class statement is typed, the user must retype "WRITE". */
  needsWriteConfirm: boolean;
  writeConfirm: string;
  onWriteConfirmChange: (next: string) => void;
  onRun: () => void;
  canRun: boolean;
  running: boolean;
};

/**
 * SQL editor chrome — single-tab variant for the pilot port. Matches
 * the pier-x-copy visual spec: file-tab style header, gutter with
 * line numbers, transparent textarea over a highlighted `<pre>` so
 * the user types directly on the tokenised output.
 */
export default function DbSqlEditor({
  tabName,
  sql,
  onChange,
  writable,
  onToggleWrite,
  needsWriteConfirm,
  writeConfirm,
  onWriteConfirmChange,
  onRun,
  canRun,
  running,
}: Props) {
  const { t } = useI18n();
  const lines = useMemo(() => sql.split("\n"), [sql]);
  const tokens = useMemo(() => renderSqlTokens(sql), [sql]);

  return (
    <div className="sq">
      <div className="sq-tabs">
        <span className="sq-tab active">
          <FileText size={10} />
          <span>{tabName ?? t("query")}</span>
        </span>
        <span className="sq-spacer" />
      </div>

      <div className="sq-editor-wrap">
        <div className="sq-gutter" aria-hidden>
          {lines.map((_, i) => (
            <div key={i} className="sq-gutter-n">
              {i + 1}
            </div>
          ))}
        </div>
        <div className="sq-editor-body">
          <pre className="sq-hl" aria-hidden>
            {tokens}
            {"\n"}
          </pre>
          <textarea
            className="sq-ta"
            value={sql}
            spellCheck={false}
            onChange={(e) => onChange(e.currentTarget.value)}
            onKeyDown={(e) => {
              if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
                e.preventDefault();
                if (canRun) onRun();
              }
            }}
          />
        </div>
      </div>

      <div className="sq-foot">
        <button
          type="button"
          className={"sq-lock" + (writable ? " on" : "")}
          onClick={onToggleWrite}
        >
          {writable ? <Unlock size={10} /> : <Lock size={10} />}
          {writable ? t("Writes unlocked") : t("Read-only")}
        </button>
        <span className="sq-foot-hint">
          {writable ? t("DML/DDL will execute.") : t("Unlock to run INSERT/UPDATE/DELETE.")}
        </span>
        {needsWriteConfirm && writable && (
          <input
            className="sq-confirm"
            value={writeConfirm}
            onChange={(e) => onWriteConfirmChange(e.currentTarget.value)}
            placeholder={t("Type WRITE to confirm")}
          />
        )}
        <span className="sq-spacer" />
        <span className="sq-shortcut">⌘↵ {t("run")}</span>
        <button
          type="button"
          className="btn is-primary is-compact"
          disabled={!canRun}
          onClick={onRun}
        >
          <Play size={10} /> {running ? t("Running...") : t("Run")}
        </button>
      </div>
    </div>
  );
}
