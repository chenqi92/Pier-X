import { KeyRound, X } from "lucide-react";
import { useEffect, useState } from "react";
import IconButton from "./IconButton";
import { useDraggableDialog } from "./useDraggableDialog";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";
import * as cmd from "../lib/commands";
import { useConnectionStore } from "../stores/useConnectionStore";

type Props = {
  open: boolean;
  onClose: () => void;
  /** SSH profile the credential is attached to. */
  savedConnectionIndex: number;
  credentialId: string;
  /** Human-readable label of the credential, shown in the title. */
  credentialLabel: string;
  /** Called after a successful password write. Panels typically
   *  retry their browse/connect here. */
  onUpdated: () => void;
};

/**
 * Password-only update for a saved DB credential. Spawned from the
 * in-banner "Update password" action when a browse/connect call
 * returns an auth error — the remote password has likely rotated and
 * the keyring copy needs a refresh. Deliberately minimal: no label /
 * host / port editing here, that's the full edit flow in
 * `DbAddCredentialDialog`.
 */
export default function DbPasswordUpdateDialog({
  open,
  onClose,
  savedConnectionIndex,
  credentialId,
  credentialLabel,
  onUpdated,
}: Props) {
  const { t } = useI18n();
  const formatError = (e: unknown) => localizeError(e, t);
  const { dialogStyle, handleProps } = useDraggableDialog(open);
  const refreshConnections = useConnectionStore((s) => s.refresh);

  const [password, setPassword] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    if (!open) return;
    setPassword("");
    setError("");
    setSaving(false);
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;

  async function save() {
    if (saving) return;
    setSaving(true);
    setError("");
    try {
      await cmd.dbCredUpdate(savedConnectionIndex, credentialId, {}, password);
      await refreshConnections();
      onUpdated();
      onClose();
    } catch (e) {
      setError(formatError(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="cmdp-overlay" onClick={onClose}>
      <div
        className="dlg"
        style={{ ...dialogStyle, maxWidth: 420 }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="dlg-head" {...handleProps}>
          <span className="dlg-title">
            <KeyRound size={13} style={{ color: "var(--accent)" }} />
            {t("Update password for {label}", { label: credentialLabel })}
          </span>
          <div style={{ flex: 1 }} />
          <IconButton variant="mini" onClick={onClose} title={t("Close")}>
            <X size={12} />
          </IconButton>
        </div>
        <div className="dlg-body dlg-body--form">
          <div className="dlg-form">
            <div className="status-note">
              {t("Enter the new password. Only the keyring entry is touched.")}
            </div>
            <label className="field-stack">
              <span className="field-label">{t("Password")}</span>
              <input
                className="field-input"
                type="password"
                value={password}
                autoFocus
                onChange={(e) => setPassword(e.currentTarget.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") void save();
                }}
              />
            </label>
            {error && (
              <div className="status-note status-note--error">{error}</div>
            )}
          </div>
        </div>
        <div className="dlg-foot">
          <div style={{ flex: 1 }} />
          <button className="gb-btn" onClick={onClose} type="button">
            {t("Cancel")}
          </button>
          <button
            className="gb-btn"
            onClick={() => void save()}
            disabled={saving}
            type="button"
          >
            {saving ? t("Saving...") : t("Save")}
          </button>
        </div>
      </div>
    </div>
  );
}
