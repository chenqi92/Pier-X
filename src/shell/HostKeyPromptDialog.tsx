// Host-key TOFU / mismatch prompt (M3b).
//
// pier-core's verifier emits a `ssh:host-key-prompt` event whenever
// it sees a host that has no pinned key yet (TOFU first contact)
// or a host whose pinned key no longer matches the presented one
// (potential MITM). The connect future blocks on a oneshot until
// `ssh_host_key_decide` delivers the user's answer, so the dialog
// is the *only* path that can either trust a fresh host or
// reinstate the connection after a key change. Closing without
// answering / timing out (3 min, enforced backend-side) counts as
// reject.

import { useEffect, useRef, useState } from "react";
import { Copy, KeyRound, ShieldAlert, X } from "lucide-react";

import * as cmd from "../lib/shellCommands";
import { useI18n } from "../i18n/useI18n";
import { toast } from "../stores/useToastStore";
import { useHostKeyPromptStore } from "../stores/useHostKeyPromptStore";

type Pending = cmd.HostKeyPromptEvent;

export default function HostKeyPromptDialog() {
  const { t } = useI18n();
  const [queue, setQueue] = useState<Pending[]>([]);
  const [busy, setBusy] = useState(false);
  // Tracks which prompt ids we've already answered so a duplicate
  // event from a backend retry can't double-fire the dialog. Backend
  // generates a fresh id per prompt so this is belt-and-braces.
  const answeredRef = useRef<Set<string>>(new Set());

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    void (async () => {
      const { listen } = await import("@tauri-apps/api/event");
      try {
        const off = await listen<Pending>("ssh:host-key-prompt", (evt) => {
          const payload = evt.payload;
          if (!payload || !payload.id || answeredRef.current.has(payload.id)) return;
          // Pause keep-alive polling panels while the user is staring
          // at the dialog. See `useHostKeyPromptStore` for why this
          // matters — the per-target gate would otherwise queue every
          // panel's invoke and discharge them all into the same React
          // batch the moment the user clicks Trust.
          useHostKeyPromptStore.getState().add(payload.id);
          setQueue((prev) =>
            prev.some((p) => p.id === payload.id) ? prev : [...prev, payload],
          );
        });
        unlisten = off;
      } catch {
        /* no listener available — silent no-op, the connect will time out */
      }
    })();
    return () => {
      if (unlisten) unlisten();
    };
  }, []);

  const current = queue[0] ?? null;

  async function decide(accept: boolean) {
    if (!current || busy) return;
    setBusy(true);
    answeredRef.current.add(current.id);
    try {
      await cmd.sshHostKeyDecide(current.id, accept);
    } catch (e) {
      toast.error(String(e));
    } finally {
      setBusy(false);
      setQueue((prev) => prev.filter((p) => p.id !== current.id));
      useHostKeyPromptStore.getState().remove(current.id);
    }
  }

  if (!current) return null;
  const { request } = current;
  const changed = request.kind === "changed";

  return (
    <div className="dlg-overlay" onClick={() => void decide(false)}>
      <div
        className="dlg dlg--host-key"
        role="dialog"
        aria-modal="true"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="dlg-head">
          {changed ? (
            <ShieldAlert size={14} className="text-neg" aria-hidden="true" />
          ) : (
            <KeyRound size={14} aria-hidden="true" />
          )}
          <span className="dlg-title">
            {changed
              ? t("Host key changed — possible MITM")
              : t("Verify host fingerprint")}
          </span>
          <div style={{ flex: 1 }} />
          <button
            type="button"
            className="lg-ic"
            disabled={busy}
            onClick={() => void decide(false)}
            aria-label={t("Reject")}
          >
            <X size={12} />
          </button>
        </div>
        <div className="dlg-body dlg-body--form">
          <div
            className={
              "status-note mono " +
              (changed ? "status-note--error" : "status-note--warn")
            }
          >
            {changed
              ? t(
                  "The pinned key for {host} no longer matches what the server presented. This is what an active man-in-the-middle would look like. Reject unless you know the host key was rotated.",
                  { host: request.host },
                )
              : t(
                  "No pinned key for {host} yet. Verify the fingerprint with the server admin out of band before accepting — this is the moment that anchors every future connection.",
                  { host: request.host },
                )}
          </div>
          <div className="dlg-row">
            <label className="dlg-row-label">{t("Host")}</label>
            <div className="mono">
              {request.host}
              <span className="muted">:{request.port}</span>
            </div>
          </div>
          <div className="dlg-row">
            <label className="dlg-row-label">{t("Key type")}</label>
            <div className="mono">{request.keyType || "—"}</div>
          </div>
          <div className="dlg-row">
            <label className="dlg-row-label">{t("Fingerprint")}</label>
            <div className="host-key-prompt__fp">
              <code className="mono">{request.fingerprint}</code>
              <button
                type="button"
                className="lg-ic"
                disabled={busy}
                onClick={() => {
                  void navigator.clipboard
                    .writeText(request.fingerprint)
                    .then(() => toast.success(t("Fingerprint copied")))
                    .catch(() => toast.error(t("Copy failed")));
                }}
                aria-label={t("Copy fingerprint")}
              >
                <Copy size={12} />
              </button>
            </div>
          </div>
          {queue.length > 1 ? (
            <div className="status-note">
              {t("{n} more prompt waiting", { n: queue.length - 1 })}
            </div>
          ) : null}
        </div>
        <div className="dlg-foot">
          <button
            type="button"
            className="btn is-ghost is-compact"
            disabled={busy}
            onClick={() => void decide(false)}
          >
            {t("Reject")}
          </button>
          <button
            type="button"
            className={
              "btn is-compact " + (changed ? "is-danger" : "is-primary")
            }
            disabled={busy}
            onClick={() => void decide(true)}
          >
            {changed ? t("Replace pin & connect") : t("Trust this host")}
          </button>
        </div>
      </div>
    </div>
  );
}
