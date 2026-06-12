import { ClipboardPaste, Plus, Shield, Trash2, X, Zap } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import IconButton from "../components/IconButton";
import { useDraggableDialog } from "../components/useDraggableDialog";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";
import { readClipboardText } from "../lib/clipboard";
import {
  egressProfileTest,
  egressWgConfSave,
  type EgressProbeResult,
} from "../lib/commands";
import { matchJumpConnection, parseEgressClipboard } from "../lib/egressImport";
import { useConnectionStore } from "../stores/useConnectionStore";
import { useEgressStore } from "../stores/useEgressStore";
import { confirm } from "../stores/useConfirmStore";
import EgressProfileForm, {
  type EgressDraft,
  egressProfileToDraft,
  emptyEgressDraft,
  persistEgressDraft,
  validateEgressDraft,
} from "./EgressProfileForm";

type Props = {
  open: boolean;
  onClose: () => void;
};

export default function EgressProfilesDialog({ open, onClose }: Props) {
  const { t } = useI18n();
  const formatError = (e: unknown) => localizeError(e, t);
  const { dialogStyle, handleProps } = useDraggableDialog(open);
  const {
    profiles,
    vpnStatus,
    refresh,
    remove,
    vpnStart,
    vpnStop,
    refreshVpnStatus,
  } = useEgressStore();
  const connections = useConnectionStore((s) => s.connections);
  const refreshConnections = useConnectionStore((s) => s.refresh);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [draft, setDraft] = useState<EgressDraft>(() => emptyEgressDraft());
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [probe, setProbe] = useState<EgressProbeResult | null>(null);
  const [probing, setProbing] = useState(false);

  useEffect(() => {
    if (open) {
      void refresh();
      void refreshConnections();
      void refreshVpnStatus();
    }
  }, [open, refresh, refreshConnections, refreshVpnStatus]);

  // Clear stale probe results whenever the user switches profiles
  // — a green check from one profile is misleading for another.
  useEffect(() => {
    setProbe(null);
  }, [selectedId, draft.kind]);

  // Reset selection when dialog opens, prefer first profile if any.
  useEffect(() => {
    if (!open) return;
    if (profiles.length > 0) {
      const first = profiles[0];
      setSelectedId(first.id);
      setDraft(egressProfileToDraft(first));
    } else {
      setSelectedId(null);
      setDraft(emptyEgressDraft());
    }
    setError("");
  }, [open, profiles.length]);

  // Esc closes.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  const sortedProfiles = useMemo(
    () => [...profiles].sort((a, b) => a.name.localeCompare(b.name)),
    [profiles],
  );

  if (!open) return null;

  function selectProfile(id: string) {
    const p = profiles.find((x) => x.id === id);
    if (!p) return;
    setSelectedId(id);
    setDraft(egressProfileToDraft(p));
    setError("");
  }

  function startNew() {
    const next = emptyEgressDraft();
    setSelectedId(null);
    setDraft(next);
    setError("");
  }

  async function handleSave() {
    if (busy) return;
    const invalid = validateEgressDraft(draft);
    if (invalid) {
      setError(t(invalid));
      return;
    }
    setBusy(true);
    try {
      const profile = await persistEgressDraft(draft);
      setError("");
      setDraft({ ...draft, isNew: false, auth: { ...draft.auth, dirty: false } });
      setSelectedId(profile.id);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
    }
  }

  async function handleVpnStart() {
    if (!selectedId || busy) return;
    setBusy(true);
    try {
      await vpnStart(selectedId);
      setError("");
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
    }
  }

  async function handleVpnStop() {
    if (!selectedId || busy) return;
    setBusy(true);
    try {
      await vpnStop(selectedId);
      setError("");
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
    }
  }

  /// Probe the *saved* version of the selected profile (so the
  /// caller can verify "the thing on disk works" rather than
  /// "the half-edited form would work"). Pre-saved profiles can
  /// still be probed without selecting one — id = null = direct.
  async function handleTest() {
    if (probing) return;
    setProbing(true);
    setProbe(null);
    try {
      const result = await egressProfileTest(selectedId, undefined, undefined);
      setProbe(result);
    } catch (e) {
      setProbe({
        ok: false,
        latencyMs: null,
        error: formatError(e),
        target: "1.1.1.1:443",
      });
    } finally {
      setProbing(false);
    }
  }

  /// Read the clipboard, sniff a wg-quick conf / SOCKS or HTTP proxy
  /// URL / OpenSSH ProxyJump, and seed `draft` with the parsed
  /// fields. WireGuard configs are written to the app-managed slot
  /// (`<data_dir>/egress/<id>.conf`) right away because that's the
  /// path `vpn_subprocess::plan_for` falls back to when `confPath`
  /// is empty — saving the profile separately would otherwise
  /// dead-end at `start VPN` with "config missing".
  async function handleImportFromClipboard() {
    if (busy) return;
    setError("");
    const raw = await readClipboardText();
    if (!raw.trim()) {
      setError(t("Clipboard is empty."));
      return;
    }
    const parsed = parseEgressClipboard(raw);
    if (!parsed) {
      setError(
        t(
          "Couldn't recognise a SOCKS5 / HTTP proxy URL, OpenSSH ProxyJump line, or wg-quick .conf in the clipboard.",
        ),
      );
      return;
    }
    const baseDraft: EgressDraft = { ...emptyEgressDraft(), name: draft.name || "" };
    if (parsed.kind === "socks5" || parsed.kind === "http") {
      const next: EgressDraft = {
        ...baseDraft,
        kind: parsed.kind,
        host: parsed.patch.host,
        port: String(parsed.patch.port),
        useAuth: parsed.patch.useAuth,
        auth: {
          user: parsed.patch.authUser,
          password: parsed.patch.authPassword,
          dirty: parsed.patch.useAuth,
        },
        name: baseDraft.name || `${parsed.kind}-${parsed.patch.host}`,
      };
      setSelectedId(null);
      setDraft(next);
      setError("");
      return;
    }
    if (parsed.kind === "ssh_jump") {
      const matched = matchJumpConnection(parsed.jumpHint, connections);
      const next: EgressDraft = {
        ...baseDraft,
        kind: "ssh_jump",
        viaConnection: matched ?? "",
        name:
          baseDraft.name ||
          `jump-${parsed.jumpHint.user ? parsed.jumpHint.user + "-" : ""}${parsed.jumpHint.host}`,
      };
      setSelectedId(null);
      setDraft(next);
      setError(
        matched
          ? ""
          : t(
              "No saved SSH connection matches {hint} — pick one from the dropdown or save it first, then re-import.",
              {
                hint: `${parsed.jumpHint.user ? parsed.jumpHint.user + "@" : ""}${parsed.jumpHint.host}:${parsed.jumpHint.port}`,
              },
            ),
      );
      return;
    }
    if (parsed.kind === "wireguard") {
      const next: EgressDraft = {
        ...baseDraft,
        kind: "wireguard",
        wgConfPath: "",
        name: baseDraft.name || `wg-${baseDraft.id.slice(-6)}`,
      };
      setBusy(true);
      try {
        await egressWgConfSave(next.id, parsed.wgConf);
        setSelectedId(null);
        setDraft(next);
        setError(
          t(
            "Imported wg-quick conf into the managed slot. Save the profile, then click Start VPN.",
          ),
        );
      } catch (e) {
        setError(formatError(e));
      } finally {
        setBusy(false);
      }
      return;
    }
  }

  async function handleDelete() {
    if (!selectedId || busy) return;
    if (!(await confirm({ message: t("Delete this egress profile? Connections that referenced it will fall back to direct."), tone: "destructive" }))) {
      return;
    }
    setBusy(true);
    try {
      await remove(selectedId);
      setError("");
      startNew();
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
    }
  }

  return createPortal(
    <div className="cmdp-overlay" onClick={onClose}>
      <div className="dlg dlg--egress" style={{ ...dialogStyle, minWidth: 640 }} onClick={(e) => e.stopPropagation()}>
        <div className="dlg-head" {...handleProps}>
          <span className="dlg-title">
            <Shield size={13} />
            {t("Egress profiles")}
          </span>
          <div style={{ flex: 1 }} />
          <IconButton variant="mini" onClick={onClose} title={t("Close")}>
            <X size={12} />
          </IconButton>
        </div>
        <div className="dlg-body" style={{ display: "grid", gridTemplateColumns: "200px 1fr", gap: "var(--sp-3)", padding: "var(--sp-3)" }}>
          <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-1)", borderRight: "1px solid var(--line)", paddingRight: "var(--sp-3)" }}>
            {sortedProfiles.length === 0 && (
              <div className="dlg-row-hint">{t("No egress profiles yet.")}</div>
            )}
            {sortedProfiles.map((p) => {
              const isVpn = p.kind === "wireguard" || p.kind === "external_vpn";
              const status = isVpn ? vpnStatus[p.id] : undefined;
              const dotColor =
                status === true
                  ? "var(--pos)"
                  : status === false
                  ? "var(--neg)"
                  : "var(--dim)";
              const dotTitle =
                status === true
                  ? t("VPN running")
                  : status === false
                  ? t("VPN started this session but exited")
                  : t("VPN never started in this session");
              return (
                <button
                  key={p.id}
                  type="button"
                  className={"dlg-opt" + (selectedId === p.id ? " active" : "")}
                  onClick={() => selectProfile(p.id)}
                  style={{ justifyContent: "flex-start", textAlign: "left" }}
                >
                  <span style={{ display: "flex", alignItems: "center", gap: "var(--sp-2)", flex: 1, minWidth: 0 }}>
                    {isVpn && (
                      <span
                        title={dotTitle}
                        style={{
                          width: 8,
                          height: 8,
                          borderRadius: "50%",
                          backgroundColor: dotColor,
                          flexShrink: 0,
                        }}
                      />
                    )}
                    <span style={{ display: "flex", flexDirection: "column", alignItems: "flex-start", gap: 2, minWidth: 0 }}>
                      <span style={{ overflow: "hidden", textOverflow: "ellipsis" }}>{p.name || p.id}</span>
                      <span className="dlg-row-hint" style={{ fontSize: "var(--size-micro)" }}>
                        {p.kind}
                        {(p.kind === "socks5" || p.kind === "http") && ` ${p.host}:${p.port}`}
                        {p.kind === "ssh_jump" && ` via ${p.viaConnection}`}
                        {p.kind === "wireguard" && ` ${p.confPath || "default"}`}
                        {p.kind === "external_vpn" && ` ${p.engine}`}
                      </span>
                    </span>
                  </span>
                </button>
              );
            })}
            <button type="button" className="gb-btn" onClick={startNew} style={{ marginTop: "var(--sp-2)", justifyContent: "center" }}>
              <Plus size={12} />
              {t("New profile")}
            </button>
          </div>

          <div className="dlg-form">
            <EgressProfileForm draft={draft} onChange={setDraft} connections={connections} />

            {error && <div className="status-note status-note--error">{error}</div>}

            {probe && (
              <div
                className={
                  "status-note " + (probe.ok ? "status-note--ok" : "status-note--error")
                }
              >
                {probe.ok
                  ? t("✓ Reached %s in %dms").replace("%s", probe.target).replace("%d", String(probe.latencyMs ?? 0))
                  : t("✗ %s failed (%dms): %s")
                      .replace("%s", probe.target)
                      .replace("%d", String(probe.latencyMs ?? 0))
                      .replace("%s", probe.error)}
              </div>
            )}
          </div>
        </div>
        <div className="dlg-foot">
          <button
            type="button"
            className="gb-btn"
            disabled={busy}
            onClick={() => void handleImportFromClipboard()}
            title={t(
              "Detect a SOCKS5/HTTP proxy URL, OpenSSH ProxyJump string, or wg-quick .conf in the clipboard and pre-fill the form.",
            )}
          >
            <ClipboardPaste size={12} />
            {t("Import from clipboard")}
          </button>
          <button
            type="button"
            className="gb-btn"
            disabled={!selectedId || busy}
            onClick={() => void handleDelete()}
          >
            <Trash2 size={12} />
            {t("Delete")}
          </button>
          <button
            type="button"
            className="gb-btn"
            disabled={probing || busy}
            onClick={() => void handleTest()}
            title={t("Probe TCP reachability through the saved profile (target: 1.1.1.1:443, 5s timeout)")}
          >
            <Zap size={12} />
            {probing ? t("Testing…") : t("Test")}
          </button>
          {(draft.kind === "wireguard" || draft.kind === "external_vpn") && (
            <>
              <button
                type="button"
                className="gb-btn"
                disabled={!selectedId || busy}
                onClick={() => void handleVpnStart()}
                title={t("Spawn the system VPN client; may prompt for admin")}
              >
                {t("Start VPN")}
              </button>
              <button
                type="button"
                className="gb-btn"
                disabled={!selectedId || busy}
                onClick={() => void handleVpnStop()}
              >
                {t("Stop VPN")}
              </button>
            </>
          )}
          <div style={{ flex: 1 }} />
          <button type="button" className="gb-btn" onClick={onClose}>{t("Close")}</button>
          <button
            type="button"
            className="gb-btn primary"
            disabled={busy}
            onClick={() => void handleSave()}
          >
            {t("Save")}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
