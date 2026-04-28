import { Activity, Command, Server, Settings, SquareTerminal } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useI18n } from "../i18n/useI18n";
import { useConnectionStore } from "../stores/useConnectionStore";
import { useRecentConnectionsStore } from "../stores/useRecentConnectionsStore";
import {
  useTerminalProfilesStore,
  type TerminalProfile,
} from "../stores/useTerminalProfilesStore";
import * as cmd from "../lib/commands";

type Props = {
  onOpenLocalTerminal: (path?: string) => void;
  onNewSsh: () => void;
  onConnectSaved: (index: number) => void;
  onOpenProfile: (profile: TerminalProfile) => void;
  onSettings: () => void;
  onCommandPalette: () => void;
  /** Open the top-level host-health dashboard tab. */
  onHostsHealth: () => void;
  version?: string;
  workspaceRoot?: string;
};

function greetingFor(hour: number): string {
  if (hour < 5) return "good evening";
  if (hour < 12) return "good morning";
  if (hour < 18) return "good afternoon";
  return "good evening";
}

function formatDateLine(now: Date, locale: string): string {
  const dateText = new Intl.DateTimeFormat(locale, {
    weekday: "long",
    month: "long",
    day: "numeric",
  }).format(now);
  const hh = String(now.getHours()).padStart(2, "0");
  const mm = String(now.getMinutes()).padStart(2, "0");
  return `${dateText} · ${hh}:${mm}`;
}

/// Tiny coloured dot showing the latest TCP-probe outcome for a
/// saved connection. Rendered before the existing Server icon in
/// each recent / saved-servers row.
///
/// `undefined` probe means "haven't checked yet" — we render a
/// neutral hollow ring so a row never visually flickers between
/// "unknown" and a status colour mid-load.
function HealthDot({
  probe,
  t,
}: {
  probe: import("../lib/commands").HostHealthReport | undefined;
  t: ReturnType<typeof useI18n>["t"];
}) {
  let cls = "welcome-health-dot welcome-health-dot--unknown";
  let label = t("Probing…");
  if (probe) {
    switch (probe.status) {
      case "online":
        cls = "welcome-health-dot welcome-health-dot--online";
        label = probe.latencyMs != null
          ? t("Online · {ms} ms", { ms: probe.latencyMs })
          : t("Online");
        break;
      case "offline":
      case "timeout":
        cls = "welcome-health-dot welcome-health-dot--offline";
        label = probe.status === "offline" ? t("Offline") : t("Timeout");
        break;
      case "error":
        cls = "welcome-health-dot welcome-health-dot--error";
        label = t("Error");
        break;
    }
  }
  return <span className={cls} aria-label={label} title={label} />;
}

function relativeTime(ts: number, locale: string): string {
  const delta = Math.max(0, Date.now() - ts);
  const m = Math.floor(delta / 60000);
  const rtf = new Intl.RelativeTimeFormat(locale, { numeric: "auto" });
  if (m < 1) return rtf.format(0, "minute");
  if (m < 60) return rtf.format(-m, "minute");
  const h = Math.floor(m / 60);
  if (h < 24) return rtf.format(-h, "hour");
  const d = Math.floor(h / 24);
  if (d < 7) return rtf.format(-d, "day");
  const w = Math.floor(d / 7);
  if (w < 5) return rtf.format(-w, "week");
  return new Intl.DateTimeFormat(locale, { month: "short", day: "numeric" }).format(new Date(ts));
}

export default function WelcomeView({
  onOpenLocalTerminal,
  onNewSsh,
  onConnectSaved,
  onOpenProfile,
  onSettings,
  onCommandPalette,
  onHostsHealth,
  version,
  workspaceRoot,
}: Props) {
  const { t, locale } = useI18n();
  const { connections } = useConnectionStore();
  const recents = useRecentConnectionsStore((s) => s.recents);
  const profiles = useTerminalProfilesStore((s) => s.profiles);
  const [now, setNow] = useState(() => new Date());

  useEffect(() => {
    const id = window.setInterval(() => setNow(new Date()), 30_000);
    return () => window.clearInterval(id);
  }, []);

  // Quick TCP probe of every saved connection so each row in the
  // recents/savers list lights up green/red the moment the user
  // sees the welcome screen. Runs once on mount and refreshes
  // every 90 s while the welcome view is visible. We deliberately
  // pick a longer cadence than the dedicated dashboard (60 s) —
  // the welcome view is a quick-glance entry point, not a
  // sustained monitoring surface, and a slower refresh keeps
  // network traffic minimal even on multi-30-host setups.
  const [probes, setProbes] = useState<Record<number, cmd.HostHealthReport>>({});
  const allIndices = useMemo(
    () => connections.map((c) => c.index),
    [connections],
  );
  useEffect(() => {
    if (allIndices.length === 0) return;
    let cancelled = false;
    const tick = () => {
      void cmd
        .hostHealthProbe({ indices: allIndices, timeoutMs: 3000 })
        .then((reports) => {
          if (cancelled) return;
          setProbes((prev) => {
            const next = { ...prev };
            for (const r of reports) {
              if (r.savedConnectionIndex >= 0) {
                next[r.savedConnectionIndex] = r;
              }
            }
            return next;
          });
        })
        .catch(() => {
          /* probe failures already surface inside per-row data; silent here */
        });
    };
    tick();
    const id = window.setInterval(tick, 90_000);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
    // We re-arm only when the saved-connection list changes — a
    // stale interval probing a removed host is harmless (the
    // backend reports `error` for unknown indices) but recreating
    // the interval also picks up newly-added hosts immediately.
  }, [allIndices.length]);

  const isMac = navigator.platform.includes("Mac");
  const mod = isMac ? "⌘" : "Ctrl+";
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const greeting = t(greetingFor(now.getHours()));
  const dateLine = formatDateLine(now, localeTag);
  const user = t("friend");
  const subtitle = `pier-x · ${version ? `${version} · ` : ""}${dateLine}`;

  // Sort weight per probe state: online (0) < unknown (1) <
  // error (2) < offline/timeout (3). Within a weight bucket the
  // existing sort key (recency for the recent list, declared
  // order for saved-servers) is preserved via a stable sort. This
  // way a user with 20 hosts where 18 are up doesn't have to
  // scroll past their unreachable ones every launch — the
  // actionable subset stays at the top of the list.
  const probeWeight = (idx: number): number => {
    const r = probes[idx];
    if (!r) return 1;
    switch (r.status) {
      case "online":
        return 0;
      case "error":
        return 2;
      case "offline":
      case "timeout":
        return 3;
    }
  };

  const recentList = connections
    .map((c) => ({ conn: c, ts: recents[c.index] ?? 0 }))
    .filter((r) => r.ts > 0)
    .sort((a, b) => {
      const wa = probeWeight(a.conn.index);
      const wb = probeWeight(b.conn.index);
      if (wa !== wb) return wa - wb;
      return b.ts - a.ts;
    })
    .slice(0, 12);

  // Saved-servers fallback list (shown when there are no recents
  // yet) gets the same online-first treatment.
  const savedSorted = useMemo(() => {
    return [...connections].sort((a, b) => {
      const wa = probeWeight(a.index);
      const wb = probeWeight(b.index);
      if (wa !== wb) return wa - wb;
      return 0; // preserve declared order within a bucket
    });
    // probeWeight closes over `probes` so include it as a dep.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [connections, probes]);

  const shellLabel = workspaceRoot
    ? `zsh · ${workspaceRoot.split(/[/\\]/).pop() || "~"}`
    : "zsh · ~/";

  return (
    <div className="welcome">
      <div className="welcome-inner">
        <h1>
          {greeting}, <span>{user}</span>.
        </h1>
        <p className="wsub">{subtitle}</p>

        <div className="welcome-grid">
          <button className="w-action" onClick={() => onOpenLocalTerminal()} type="button">
            <div className="wic"><SquareTerminal size={17} /></div>
            <div className="wbody">
              <div className="wt">{t("New local terminal")}</div>
              <div className="wm">{shellLabel}</div>
            </div>
            <div className="wk">{mod}T</div>
          </button>
          <button className="w-action" onClick={onNewSsh} type="button">
            <div className="wic"><Server size={17} /></div>
            <div className="wbody">
              <div className="wt">{t("New SSH connection")}</div>
              <div className="wm">{t("saved or ad-hoc")}</div>
            </div>
            <div className="wk">{mod}N</div>
          </button>
          <button className="w-action" onClick={onCommandPalette} type="button">
            <div className="wic"><Command size={17} /></div>
            <div className="wbody">
              <div className="wt">{t("Command palette")}</div>
              <div className="wm">{t("search every action")}</div>
            </div>
            <div className="wk">{mod}K</div>
          </button>
          <button className="w-action" onClick={onHostsHealth} type="button">
            <div className="wic"><Activity size={17} /></div>
            <div className="wbody">
              <div className="wt">{t("Host health")}</div>
              <div className="wm">{t("reachability across saved hosts")}</div>
            </div>
            <div className="wk" />
          </button>
          <button className="w-action" onClick={onSettings} type="button">
            <div className="wic"><Settings size={17} /></div>
            <div className="wbody">
              <div className="wt">{t("Settings")}</div>
              <div className="wm">{t("theme, fonts, shortcuts")}</div>
            </div>
            <div className="wk">{mod},</div>
          </button>
        </div>

        {profiles.length > 0 ? (
          <div className="welcome-recent">
            <h4>{t("Terminal profiles")}</h4>
            <div className="welcome-recent-list">
              {profiles.map((profile) => (
                <button
                  key={profile.id}
                  className="recent-row"
                  onClick={() => onOpenProfile(profile)}
                  type="button"
                >
                  <SquareTerminal size={13} />
                  <span className="rname">{profile.name}</span>
                  <span className="raddr">
                    {profile.cwd || ""}
                    {profile.cwd && profile.startupCommand ? " · " : ""}
                    {profile.startupCommand || ""}
                  </span>
                  <span className="rdate">—</span>
                </button>
              ))}
            </div>
          </div>
        ) : null}

        {recentList.length > 0 ? (
          <div className="welcome-recent">
            <h4>{t("Recent connections")}</h4>
            <div className="welcome-recent-list">
              {recentList.map(({ conn, ts }) => (
                <button
                  key={conn.index}
                  className="recent-row"
                  onClick={() => onConnectSaved(conn.index)}
                  type="button"
                >
                  <HealthDot probe={probes[conn.index]} t={t} />
                  <Server size={13} />
                  <span className="rname">{conn.name || `${conn.user}@${conn.host}`}</span>
                  <span className="raddr">{conn.user}@{conn.host}{conn.port !== 22 ? `:${conn.port}` : ""}</span>
                  <span className="rdate">{relativeTime(ts, localeTag)}</span>
                </button>
              ))}
            </div>
          </div>
        ) : connections.length > 0 ? (
          <div className="welcome-recent">
            <h4>{t("Saved servers")}</h4>
            <div className="welcome-recent-list">
              {savedSorted.slice(0, 12).map((conn) => (
                <button
                  key={conn.index}
                  className="recent-row"
                  onClick={() => onConnectSaved(conn.index)}
                  type="button"
                >
                  <HealthDot probe={probes[conn.index]} t={t} />
                  <Server size={13} />
                  <span className="rname">{conn.name || `${conn.user}@${conn.host}`}</span>
                  <span className="raddr">{conn.user}@{conn.host}{conn.port !== 22 ? `:${conn.port}` : ""}</span>
                  <span className="rdate">—</span>
                </button>
              ))}
            </div>
          </div>
        ) : null}
      </div>
    </div>
  );
}
