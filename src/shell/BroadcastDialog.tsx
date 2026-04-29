// One-shot fanout dispatcher for typing the same command into every
// selected SSH session. PuTTY-tray / iTerm-broadcast in spirit, but
// delivered as a dialog rather than a persistent input mode — keeps
// the regular per-tab typing path unchanged so a stray "broadcast on"
// flag can't silently push commands into prod.
//
// Lives at the App layer because it needs the global tab list and
// `terminalWrite` IPC. Dialog opens from the title-bar Session menu.

import { useEffect, useMemo, useState } from "react";
import { Send, X } from "lucide-react";

import * as cmd from "../lib/commands";
import { useI18n } from "../i18n/useI18n";
import { useTabStore } from "../stores/useTabStore";
import { toast } from "../stores/useToastStore";

type Props = {
  open: boolean;
  onClose: () => void;
  /** Optional pre-selection. When set, only these tab ids are
   *  default-checked (intersected with live SSH tabs). Used by the
   *  sidebar's "Broadcast to group" so opening from a group menu
   *  doesn't default to fanning out to your unrelated prod fleet
   *  tabs. Null / undefined = default behaviour (all live SSH). */
  prefilterTabIds?: string[] | null;
};

export default function BroadcastDialog({
  open,
  onClose,
  prefilterTabIds,
}: Props) {
  const { t } = useI18n();
  const tabs = useTabStore((s) => s.tabs);

  // Eligible targets: SSH-backed tabs with a live PTY session id.
  // Local terminals are excluded — broadcasting to a local session
  // would type into your laptop's shell, which is rarely the
  // intent. Tabs whose PTY hasn't booted yet show up disabled.
  const candidates = useMemo(
    () =>
      tabs
        .filter((t) => t.backend === "ssh")
        .map((t) => ({
          id: t.id,
          name: t.title,
          target: `${t.sshUser}@${t.sshHost}:${t.sshPort}`,
          live: !!t.terminalSessionId,
          sessionId: t.terminalSessionId,
        })),
    [tabs],
  );

  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [command, setCommand] = useState("");
  const [appendNewline, setAppendNewline] = useState(true);
  const [sending, setSending] = useState(false);

  // Default-checked set on open. Without a prefilter we pick every
  // live SSH tab — the fleet-ops happy path. With a prefilter, only
  // live tabs whose id is in the prefilter list are checked, so
  // "Broadcast to group" doesn't accidentally fan out to unrelated
  // prod tabs that happen to be open.
  useEffect(() => {
    if (!open) return;
    const live = candidates.filter((c) => c.live);
    const fresh = new Set<string>();
    if (prefilterTabIds && prefilterTabIds.length > 0) {
      const allow = new Set(prefilterTabIds);
      for (const c of live) if (allow.has(c.id)) fresh.add(c.id);
    } else {
      for (const c of live) fresh.add(c.id);
    }
    setPicked(fresh);
    setCommand("");
  }, [open, candidates, prefilterTabIds]);

  if (!open) return null;

  const liveCount = candidates.filter((c) => c.live).length;

  async function send() {
    const targets = candidates.filter(
      (c) => c.live && c.sessionId && picked.has(c.id),
    );
    if (targets.length === 0) {
      toast.warn(t("Pick at least one live SSH tab."));
      return;
    }
    if (!command.trim()) {
      toast.warn(t("Type a command first."));
      return;
    }
    // Append-newline triggers immediate execution at the remote
    // shell. Off = land at the prompt so the user can review on
    // each session before pressing Enter manually. Useful for
    // parameterised commands they want to confirm per-host.
    const text = appendNewline ? `${command}\n` : command;
    setSending(true);
    let ok = 0;
    let bad = 0;
    for (const t of targets) {
      if (!t.sessionId) continue;
      try {
        await cmd.terminalWrite(t.sessionId, text);
        ok += 1;
      } catch {
        bad += 1;
      }
    }
    setSending(false);
    toast.info(
      t("Broadcast sent: {ok} ok, {bad} failed", { ok, bad }),
    );
    onClose();
  }

  return (
    <div className="dlg-overlay" onClick={onClose}>
      <div
        className="dlg dlg--broadcast"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="dlg-head">
          <span className="dlg-title">{t("Broadcast to terminals")}</span>
          <div style={{ flex: 1 }} />
          <button type="button" className="lg-ic" onClick={onClose}>
            <X size={12} />
          </button>
        </div>
        <div className="dlg-body dlg-body--form">
          {liveCount === 0 ? (
            <div className="status-note mono">
              {t(
                "No live SSH tabs. Open a connection first, then come back.",
              )}
            </div>
          ) : (
            <>
              <div className="dlg-row">
                <label className="dlg-row-label">{t("Targets")}</label>
                <div className="broadcast-targets">
                  {candidates.map((c) => (
                    <label
                      key={c.id}
                      className={
                        "broadcast-target mono" +
                        (c.live ? "" : " is-disabled")
                      }
                    >
                      <input
                        type="checkbox"
                        checked={picked.has(c.id)}
                        disabled={!c.live}
                        onChange={(e) => {
                          setPicked((prev) => {
                            const next = new Set(prev);
                            if (e.currentTarget.checked) next.add(c.id);
                            else next.delete(c.id);
                            return next;
                          });
                        }}
                      />
                      <span className="broadcast-target__name">
                        {c.name}
                      </span>
                      <span className="broadcast-target__addr muted">
                        {c.target}
                      </span>
                      {!c.live && (
                        <span className="broadcast-target__note">
                          {t("(not connected)")}
                        </span>
                      )}
                    </label>
                  ))}
                </div>
              </div>
              <div className="dlg-row">
                <label className="dlg-row-label">{t("Command")}</label>
                <textarea
                  className="dlg-input mono"
                  rows={3}
                  spellCheck={false}
                  value={command}
                  placeholder={t(
                    "e.g. uptime ; df -h ; sudo journalctl -u nginx --since '5 min ago'",
                  )}
                  onChange={(e) => setCommand(e.currentTarget.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                      e.preventDefault();
                      void send();
                    }
                  }}
                />
              </div>
              <label className="dlg-row broadcast-flag">
                <input
                  type="checkbox"
                  checked={appendNewline}
                  onChange={(e) => setAppendNewline(e.currentTarget.checked)}
                />
                <span>
                  {t(
                    "Append newline (run immediately on each host).",
                  )}
                </span>
              </label>
              <div className="status-note status-note--warn mono">
                {t(
                  "Verify before sending — this fans the same command into every checked tab. ⌘/Ctrl+Enter to send.",
                )}
              </div>
            </>
          )}
        </div>
        <div className="dlg-foot">
          <button
            type="button"
            className="btn is-ghost is-compact"
            onClick={onClose}
          >
            {t("Cancel")}
          </button>
          <button
            type="button"
            className="btn is-primary is-compact"
            disabled={sending || picked.size === 0 || !command.trim()}
            onClick={() => void send()}
          >
            <Send size={10} />
            {sending
              ? t("Sending…")
              : t("Send to {n}", { n: picked.size })}
          </button>
        </div>
      </div>
    </div>
  );
}
