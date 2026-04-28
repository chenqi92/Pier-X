import { useEffect, useMemo, useState } from "react";
import { Globe, RefreshCw } from "lucide-react";
import * as cmd from "../lib/commands";
import type {
  WebServerDetection,
  WebServerInfo,
  WebServerKind,
} from "../lib/commands";
import PanelHeader from "../components/PanelHeader";
import PanelSkeleton, { useDeferredMount } from "../components/PanelSkeleton";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";
import {
  effectiveSshTarget,
  isSshTargetReady,
  type TabState,
} from "../lib/types";
import NginxPanel from "./NginxPanel";
import RawWebServerPanel from "./RawWebServerPanel";

// One unified entry for nginx / apache / caddy. nginx routes to the
// rich NginxPanel; apache and caddy land on a placeholder that exposes
// version, loaded-modules summary, and the validate / reload buttons —
// enough to be useful while structured editing for those products is
// still on the roadmap.

type Props = { tab: TabState | null };

export default function WebServerPanel(props: Props) {
  const ready = useDeferredMount();
  return (
    <div className="panel-stage">
      {ready ? (
        <WebServerPanelBody {...props} />
      ) : (
        <PanelSkeleton variant="rows" rows={6} />
      )}
    </div>
  );
}

function WebServerPanelBody({ tab }: Props) {
  const { t } = useI18n();
  const formatError = (e: unknown) => localizeError(e, t);

  const sshTarget = tab ? effectiveSshTarget(tab) : null;
  const canProbe = isSshTargetReady(sshTarget);

  const sshParams = useMemo(() => {
    if (!sshTarget) return null;
    return {
      host: sshTarget.host,
      port: sshTarget.port,
      user: sshTarget.user,
      authMode: sshTarget.authMode,
      password: sshTarget.password,
      keyPath: sshTarget.keyPath,
      savedConnectionIndex: sshTarget.savedConnectionIndex,
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    sshTarget?.host,
    sshTarget?.port,
    sshTarget?.user,
    sshTarget?.authMode,
    sshTarget?.password,
    sshTarget?.keyPath,
    sshTarget?.savedConnectionIndex,
  ]);

  const [detection, setDetection] = useState<WebServerDetection | null>(null);
  const [detectBusy, setDetectBusy] = useState(false);
  const [detectError, setDetectError] = useState("");
  const [activeKind, setActiveKind] = useState<WebServerKind | null>(null);

  useEffect(() => {
    if (!sshParams || !canProbe) {
      setDetection(null);
      return;
    }
    let cancelled = false;
    setDetectBusy(true);
    setDetectError("");
    cmd
      .webServerDetect(sshParams)
      .then((result) => {
        if (cancelled) return;
        setDetection(result);
        // Default to the first detected product (nginx wins on ties
        // because the backend orders detection in nginx → apache → caddy).
        const first = result.detected[0];
        setActiveKind((prev) => prev ?? first?.kind ?? null);
      })
      .catch((e) => {
        if (cancelled) return;
        setDetectError(formatError(e));
      })
      .finally(() => {
        if (!cancelled) setDetectBusy(false);
      });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sshParams?.host, sshParams?.port, sshParams?.user, canProbe]);

  // Re-pick a sensible default if the active kind disappears (e.g. a
  // probe re-runs after the user uninstalled apache).
  useEffect(() => {
    if (!detection) return;
    if (activeKind && detection.detected.some((d) => d.kind === activeKind)) {
      return;
    }
    setActiveKind(detection.detected[0]?.kind ?? null);
  }, [detection, activeKind]);

  if (!tab) {
    return (
      <div className="panel-section panel-section--empty">
        <div className="panel-section__title mono">
          <Globe size={12} /> {t("Web Server")}
        </div>
        <div className="status-note mono">
          {t("Open an SSH tab to manage the web server.")}
        </div>
      </div>
    );
  }
  if (!sshTarget) {
    return (
      <div className="panel-section panel-section--empty">
        <div className="panel-section__title mono">
          <Globe size={12} /> {t("Web Server")}
        </div>
        <div className="status-note mono">
          {t("This tab has no SSH context — web server management is remote-only.")}
        </div>
      </div>
    );
  }

  // Probe in flight on first mount — show a thin status bar but don't
  // block. Once detection arrives we either route to NginxPanel or
  // render the placeholder.
  const detected = detection?.detected ?? [];
  const activeInfo = activeKind
    ? detected.find((d) => d.kind === activeKind) ?? null
    : null;

  // Loading-first-time view.
  if (!detection && detectBusy) {
    return (
      <>
        <PanelHeader
          icon={Globe}
          title={t("Web Server")}
          meta={`${sshTarget.user}@${sshTarget.host}`}
        />
        <div className="status-note mono">{t("Detecting web servers…")}</div>
      </>
    );
  }

  // Detection failed outright.
  if (detectError && !detection) {
    return (
      <>
        <PanelHeader
          icon={Globe}
          title={t("Web Server")}
          meta={`${sshTarget.user}@${sshTarget.host}`}
          actions={
            <button
              type="button"
              className="btn btn--ghost btn--icon"
              title={t("Retry")}
              onClick={() => {
                setDetectError("");
                if (sshParams) {
                  setDetectBusy(true);
                  cmd
                    .webServerDetect(sshParams)
                    .then(setDetection)
                    .catch((e) => setDetectError(formatError(e)))
                    .finally(() => setDetectBusy(false));
                }
              }}
            >
              <RefreshCw size={11} />
            </button>
          }
        />
        <div className="status-note mono status-note--error">{detectError}</div>
      </>
    );
  }

  // No web server installed.
  if (detected.length === 0) {
    return (
      <>
        <PanelHeader
          icon={Globe}
          title={t("Web Server")}
          meta={`${sshTarget.user}@${sshTarget.host}`}
        />
        <div className="ws-empty">
          <div className="ws-empty__title mono">
            {t("No web server detected")}
          </div>
          <div className="ws-empty__hint">
            {t(
              "Install nginx, Apache, or Caddy via the Software panel — Pier-X will pick it up automatically.",
            )}
          </div>
        </div>
      </>
    );
  }

  // nginx routes straight through to the existing rich panel — keeps
  // identity (URL state, dirty buffers) when the user toggles the
  // segmented control between products on multi-install hosts.
  if (activeKind === "nginx") {
    return (
      <div className="ws-panel">
        {detected.length > 1 && (
          <ProductSegmented
            detected={detected}
            active={activeKind}
            onPick={setActiveKind}
            t={t}
          />
        )}
        <NginxPanel tab={tab} />
      </div>
    );
  }

  // apache / caddy → raw editor with the standard save → validate →
  // reload pipeline.
  return (
    <div className="ws-panel">
      {detected.length > 1 && (
        <ProductSegmented
          detected={detected}
          active={activeKind!}
          onPick={setActiveKind}
          t={t}
        />
      )}
      {activeInfo && sshParams && activeKind && (
        <RawWebServerPanel
          key={`${activeKind}-${sshParams.host}`}
          kind={activeKind}
          sshParams={sshParams}
        />
      )}
    </div>
  );
}

// ── Segmented product picker ────────────────────────────────────────

function ProductSegmented({
  detected,
  active,
  onPick,
  t,
}: {
  detected: WebServerInfo[];
  active: WebServerKind;
  onPick: (k: WebServerKind) => void;
  t: (s: string) => string;
}) {
  return (
    <div className="ws-segmented" role="tablist">
      {detected.map((d) => (
        <button
          key={d.kind}
          type="button"
          role="tab"
          aria-selected={d.kind === active}
          className={`ws-segmented__btn ${d.kind === active ? "is-active" : ""}`}
          onClick={() => onPick(d.kind)}
          title={d.version || d.binary}
        >
          <span className="ws-segmented__label mono">{productLabel(d.kind, t)}</span>
          {d.running === "active" && (
            <span className="ws-segmented__dot is-on" title={t("Running")} />
          )}
          {d.running === "inactive" && (
            <span className="ws-segmented__dot is-off" title={t("Stopped")} />
          )}
        </button>
      ))}
    </div>
  );
}

function productLabel(kind: WebServerKind, t: (s: string) => string): string {
  switch (kind) {
    case "nginx":
      return "nginx";
    case "apache":
      return t("Apache");
    case "caddy":
      return "Caddy";
  }
}

