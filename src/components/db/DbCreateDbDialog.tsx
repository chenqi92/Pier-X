import { Database, X } from "lucide-react";
import { useEffect, useState } from "react";

import IconButton from "../IconButton";
import { useDraggableDialog } from "../useDraggableDialog";
import { useI18n } from "../../i18n/useI18n";

type Props = {
  open: boolean;
  /** Engine flavor — drives which optional fields render
   *  (`charset`/`collation` for MySQL, `owner` for Postgres). */
  kind: "mysql" | "postgres";
  onCancel: () => void;
  onSubmit: (
    name: string,
    options: { charset?: string; collation?: string; owner?: string },
  ) => Promise<void>;
};

/**
 * Minimal "Create database" dialog for the schema-tree right-click
 * action. Keeps the form intentionally short — name plus one or two
 * optional knobs per engine. Power users still drop into the SQL
 * editor for finer control (replication options, tablespaces, etc.).
 */
export default function DbCreateDbDialog({ open, kind, onCancel, onSubmit }: Props) {
  const { t } = useI18n();
  const { dialogStyle, handleProps } = useDraggableDialog(open);
  const [name, setName] = useState("");
  const [charset, setCharset] = useState("");
  const [collation, setCollation] = useState("");
  const [owner, setOwner] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    if (!open) {
      setName("");
      setCharset("");
      setCollation("");
      setOwner("");
      setBusy(false);
      setError("");
    }
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onCancel]);

  if (!open) return null;

  const submit = async () => {
    const trimmed = name.trim();
    if (!trimmed) {
      setError(t("Database name is required."));
      return;
    }
    setBusy(true);
    setError("");
    try {
      await onSubmit(trimmed, {
        charset: charset.trim() || undefined,
        collation: collation.trim() || undefined,
        owner: owner.trim() || undefined,
      });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="cmdp-overlay" onClick={onCancel}>
      <div
        className="dlg"
        style={{ ...dialogStyle, maxWidth: 460 }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="dlg-head" {...handleProps}>
          <span className="dlg-title">
            <Database size={13} style={{ color: "var(--accent)" }} />
            {t("New database")}
          </span>
          <div style={{ flex: 1 }} />
          <IconButton variant="mini" onClick={onCancel} title={t("Close")}>
            <X size={12} />
          </IconButton>
        </div>
        <div className="dlg-body dlg-body--form">
          <div className="dlg-form">
            <label className="dlg-row">
              <span className="dlg-label">{t("Name")}</span>
              <input
                className="dlg-input"
                value={name}
                placeholder="my_database"
                autoFocus
                onChange={(e) => setName(e.currentTarget.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") void submit();
                }}
              />
            </label>
            {kind === "mysql" && (
              <>
                <label className="dlg-row">
                  <span className="dlg-label">{t("Charset")}</span>
                  <input
                    className="dlg-input"
                    value={charset}
                    placeholder="utf8mb4"
                    onChange={(e) => setCharset(e.currentTarget.value)}
                  />
                </label>
                <label className="dlg-row">
                  <span className="dlg-label">{t("Collation")}</span>
                  <input
                    className="dlg-input"
                    value={collation}
                    placeholder="utf8mb4_unicode_ci"
                    onChange={(e) => setCollation(e.currentTarget.value)}
                  />
                </label>
              </>
            )}
            {kind === "postgres" && (
              <label className="dlg-row">
                <span className="dlg-label">{t("Owner")}</span>
                <input
                  className="dlg-input"
                  value={owner}
                  placeholder={t("(current role)")}
                  onChange={(e) => setOwner(e.currentTarget.value)}
                />
              </label>
            )}
            {error && <div className="status-note status-note--error">{error}</div>}
          </div>
        </div>
        <div className="dlg-foot">
          <div style={{ flex: 1 }} />
          <button className="gb-btn" onClick={onCancel} type="button" disabled={busy}>
            {t("Cancel")}
          </button>
          <button
            className="gb-btn gb-btn--primary"
            onClick={() => void submit()}
            type="button"
            disabled={busy}
          >
            {busy ? t("Creating…") : t("Create")}
          </button>
        </div>
      </div>
    </div>
  );
}
