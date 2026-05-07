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
  /** Initial state of the "记住此主机的提权密码" checkbox.
   *  Defaults to `true` so the friendly path is the default — once
   *  the user has typed the password, next time the panel can
   *  resolve it from the keychain without prompting. Set to `false`
   *  on shared / borrowed machines. */
  defaultRemember?: boolean;
  /** Hide the "记住" checkbox entirely. Use for callers that
   *  intentionally only want a one-shot prompt (e.g. an explicit
   *  "Forget password" → re-enter flow). */
  hideRemember?: boolean;
  onSubmit: (password: string, remember: boolean) => void;
  onCancel: () => void;
};

/**
 * Modal password prompt for sudo escalation. Panels that hit a
 * `permission-denied` / `sudo-requires-password` outcome pop this
 * to ask the user for their host password. Submitting puts the
 * value into the in-memory `useSudoStore` cache; if "记住" is
 * checked, also persists to the OS keychain so the next launch
 * can skip the prompt entirely.
 */
export default function SudoPasswordDialog({
  open,
  hostLabel,
  errorMessage,
  busy = false,
  defaultRemember = true,
  hideRemember = false,
  onSubmit,
  onCancel,
}: Props) {
  const { t } = useI18n();
  const inputRef = useRef<HTMLInputElement>(null);
  const [password, setPassword] = useState("");
  const [remember, setRemember] = useState(defaultRemember);

  // Reset the field every time the dialog opens — and clear
  // whatever was typed last time so a previous wrong attempt
  // doesn't pre-fill on the next prompt.
  useEffect(() => {
    if (open) {
      setPassword("");
      setRemember(defaultRemember);
      // Defer one frame so Dialog's own focus logic settles first.
      requestAnimationFrame(() => inputRef.current?.focus());
    }
  }, [open, defaultRemember]);

  function commit() {
    if (busy) return;
    if (!password) return;
    onSubmit(password, remember);
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
          {remember
            ? t(
                "This host requires a password for sudo. With \"Remember\" checked, the password is saved in your OS keychain so the next session can skip this prompt.",
              )
            : t(
                "This host requires a password for sudo. The password stays in memory for this session only.",
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
        {hideRemember ? null : (
          <label
            style={{
              display: "inline-flex",
              alignItems: "center",
              gap: "var(--sp-2)",
              fontSize: "var(--ui-fs-sm)",
            }}
          >
            <input
              type="checkbox"
              checked={remember}
              disabled={busy}
              onChange={(e) => setRemember(e.currentTarget.checked)}
            />
            <span>{t("Remember the elevation password for this host")}</span>
          </label>
        )}
      </div>
    </Dialog>
  );
}
