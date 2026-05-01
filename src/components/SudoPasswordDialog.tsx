import { KeyRound } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import Dialog from "./Dialog";
import { useI18n } from "../i18n/useI18n";

type Props = {
  open: boolean;
  /** Host shown in the subtitle, e.g. `chenqi@192.168.0.10`. */
  hostLabel: string;
  /** When set, shown above the input — used to convey "wrong
   *  password, please try again" between attempts. */
  errorMessage?: string;
  /** `true` while the parent is busy retrying — disables the input
   *  and shows a spinner-style state on the submit button. */
  busy?: boolean;
  onSubmit: (password: string) => void;
  onCancel: () => void;
};

/**
 * Modal password prompt for sudo escalation. The Software panel
 * pops this when an install / uninstall / mirror / compose action
 * comes back with `status: "sudo-requires-password"`. Submitting
 * caches the password per-host (in memory only) and triggers the
 * panel to re-issue the same command with `sudoPassword` filled
 * in. Cancelling lets the original `sudo-requires-password`
 * outcome stand so the user can fix sudoers themselves.
 */
export default function SudoPasswordDialog({
  open,
  hostLabel,
  errorMessage,
  busy = false,
  onSubmit,
  onCancel,
}: Props) {
  const { t } = useI18n();
  const inputRef = useRef<HTMLInputElement>(null);
  const [password, setPassword] = useState("");

  // Reset the field every time the dialog opens — and clear
  // whatever was typed last time so a previous wrong attempt
  // doesn't pre-fill on the next prompt.
  useEffect(() => {
    if (open) {
      setPassword("");
      // Defer one frame so Dialog's own focus logic settles first.
      requestAnimationFrame(() => inputRef.current?.focus());
    }
  }, [open]);

  function commit() {
    if (busy) return;
    if (!password) return;
    onSubmit(password);
  }

  return (
    <Dialog
      open={open}
      title={
        <span style={{ display: "inline-flex", alignItems: "center", gap: "var(--sp-2)" }}>
          <KeyRound size={14} />
          {t("Sudo password required")}
        </span>
      }
      subtitle={hostLabel}
      size="sm"
      closeOnOverlay={!busy}
      closeOnEscape={!busy}
      onClose={() => {
        if (!busy) onCancel();
      }}
      footer={
        <>
          <button
            type="button"
            className="btn"
            onClick={onCancel}
            disabled={busy}
          >
            {t("Cancel")}
          </button>
          <button
            type="button"
            className="btn is-primary"
            onClick={commit}
            disabled={busy || !password}
          >
            {busy ? t("Authenticating…") : t("OK")}
          </button>
        </>
      }
    >
      <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-3)" }}>
        <p className="muted" style={{ margin: 0 }}>
          {t(
            "This host requires a password for sudo. The password stays in memory for this session and is never written to disk.",
          )}
        </p>
        {errorMessage ? (
          <div
            className="banner banner--error"
            role="alert"
            style={{ margin: 0 }}
          >
            {errorMessage}
          </div>
        ) : null}
        <label
          style={{ display: "flex", flexDirection: "column", gap: "var(--sp-1)" }}
        >
          <span className="muted" style={{ fontSize: "var(--ui-fs-sm)" }}>
            {t("Password")}
          </span>
          <input
            ref={inputRef}
            type="password"
            className="input mono"
            autoComplete="off"
            spellCheck={false}
            value={password}
            disabled={busy}
            onChange={(e) => setPassword(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                commit();
              }
            }}
          />
        </label>
      </div>
    </Dialog>
  );
}
