import { useState } from "react";
import { Loader2, Sparkles } from "lucide-react";
import { useI18n } from "../../i18n/useI18n";
import { useSettingsStore } from "../../stores/useSettingsStore";
import { generateSql, type SqlSchemaContext } from "../../lib/aiSql";

// Reusable "describe → SQL" affordance for the lighter custom-grid panels
// (SQL Server / InfluxDB / Oracle / Dameng) that use a plain textarea
// editor rather than the shared DbSqlEditor. Renders a compact ✨ button
// that expands inline into a prompt; on success it hands the generated
// SQL back to the panel via `onResult`.

type Props = {
  /** Schema context, resolved lazily so it reflects state at click time. */
  schema: SqlSchemaContext | (() => SqlSchemaContext);
  onResult: (sql: string) => void;
};

export default function DbAiGenerate({ schema, onResult }: Props) {
  const { t } = useI18n();
  const settings = useSettingsStore();
  const [open, setOpen] = useState(false);
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");

  const run = async () => {
    const desc = text.trim();
    if (!desc) return;
    if (!settings.aiModel || !settings.aiProviderKind) {
      setErr(t("Configure an AI provider in Settings → AI first."));
      return;
    }
    setBusy(true);
    setErr("");
    try {
      const sql = await generateSql({
        provider: {
          kind: settings.aiProviderKind,
          baseUrl: settings.aiBaseUrl,
          model: settings.aiModel,
          maxTokens: settings.aiMaxTokens > 0 ? settings.aiMaxTokens : null,
          secretId: settings.aiVendorId,
        },
        schema: typeof schema === "function" ? schema() : schema,
        description: desc,
      });
      if (sql.trim()) {
        onResult(sql.trim());
        setOpen(false);
        setText("");
      } else {
        setErr(t("The model returned no SQL."));
      }
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  if (!open) {
    return (
      <button
        type="button"
        className="btn is-ghost is-compact dbq-ai-btn"
        onClick={() => {
          setErr("");
          setOpen(true);
        }}
        title={t("Generate SQL with AI")}
      >
        <Sparkles size={11} /> AI
      </button>
    );
  }

  return (
    <span className="dbq-ai">
      <Sparkles size={11} className="dbq-ai__glyph" />
      <input
        className="dbq-ai__input mono"
        value={text}
        autoFocus
        placeholder={t("Describe the query in plain language…")}
        disabled={busy}
        onChange={(e) => setText(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            void run();
          } else if (e.key === "Escape") {
            setOpen(false);
          }
        }}
      />
      <button
        type="button"
        className="btn is-primary is-compact"
        disabled={busy || !text.trim()}
        onClick={() => void run()}
      >
        {busy ? <Loader2 size={11} className="spin" /> : <Sparkles size={11} />}
        {busy ? t("Generating…") : t("Generate")}
      </button>
      {err && <span className="dbq-ai__err mono">{err}</span>}
    </span>
  );
}
