import { Command, Server, Settings, SquareTerminal } from "lucide-react";
import { useEffect, useState } from "react";
import { useI18n } from "../i18n/useI18n";
import { useConnectionStore } from "../stores/useConnectionStore";
import { useRecentConnectionsStore } from "../stores/useRecentConnectionsStore";

type Props = {
  onOpenLocalTerminal: (path?: string) => void;
  onNewSsh: () => void;
  onConnectSaved: (index: number) => void;
  onSettings: () => void;
  onCommandPalette: () => void;
  version?: string;
  workspaceRoot?: string;
};

function greetingFor(hour: number): string {
  if (hour < 5) return "good evening";
  if (hour < 12) return "good morning";
  if (hour < 18) return "good afternoon";
  return "good evening";
}

function formatDateLine(now: Date): string {
  const weekday = now.toLocaleDateString(undefined, { weekday: "long" });
  const month = now.toLocaleDateString(undefined, { month: "long" });
  const day = now.getDate();
  const hh = String(now.getHours()).padStart(2, "0");
  const mm = String(now.getMinutes()).padStart(2, "0");
  return `${weekday.toLowerCase()}, ${month.toLowerCase()} ${day} · ${hh}:${mm}`;
}

function relativeTime(ts: number): string {
  const delta = Math.max(0, Date.now() - ts);
  const m = Math.floor(delta / 60000);
  if (m < 1) return "just now";
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ago`;
  const d = Math.floor(h / 24);
  if (d < 2) return "yesterday";
  if (d < 7) return `${d}d ago`;
  const w = Math.floor(d / 7);
  if (w < 5) return `${w}w ago`;
  return new Date(ts).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

export default function WelcomeView({
  onOpenLocalTerminal,
  onNewSsh,
  onConnectSaved,
  onSettings,
  onCommandPalette,
  version,
  workspaceRoot,
}: Props) {
  const { t } = useI18n();
  const { connections } = useConnectionStore();
  const recents = useRecentConnectionsStore((s) => s.recents);
  const [now, setNow] = useState(() => new Date());

  useEffect(() => {
    const id = window.setInterval(() => setNow(new Date()), 30_000);
    return () => window.clearInterval(id);
  }, []);

  const isMac = navigator.platform.includes("Mac");
  const mod = isMac ? "⌘" : "Ctrl+";
  const greeting = greetingFor(now.getHours());
  const dateLine = formatDateLine(now);
  const user = "you";
  const subtitle = `pier-x · ${version ? `${version} · ` : ""}${dateLine}`;

  const recentList = connections
    .map((c) => ({ conn: c, ts: recents[c.index] ?? 0 }))
    .filter((r) => r.ts > 0)
    .sort((a, b) => b.ts - a.ts)
    .slice(0, 5);

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
          <button className="w-action" onClick={onSettings} type="button">
            <div className="wic"><Settings size={17} /></div>
            <div className="wbody">
              <div className="wt">{t("Settings")}</div>
              <div className="wm">{t("theme, fonts, shortcuts")}</div>
            </div>
            <div className="wk">{mod},</div>
          </button>
        </div>

        {recentList.length > 0 ? (
          <div className="welcome-recent">
            <h4>{t("Recent connections")}</h4>
            {recentList.map(({ conn, ts }) => (
              <button
                key={conn.index}
                className="recent-row"
                onClick={() => onConnectSaved(conn.index)}
                type="button"
              >
                <Server size={13} />
                <span className="rname">{conn.name || `${conn.user}@${conn.host}`}</span>
                <span className="raddr">{conn.user}@{conn.host}{conn.port !== 22 ? `:${conn.port}` : ""}</span>
                <span className="rdate">{relativeTime(ts)}</span>
              </button>
            ))}
          </div>
        ) : connections.length > 0 ? (
          <div className="welcome-recent">
            <h4>{t("Saved servers")}</h4>
            {connections.slice(0, 5).map((conn) => (
              <button
                key={conn.index}
                className="recent-row"
                onClick={() => onConnectSaved(conn.index)}
                type="button"
              >
                <Server size={13} />
                <span className="rname">{conn.name || `${conn.user}@${conn.host}`}</span>
                <span className="raddr">{conn.user}@{conn.host}{conn.port !== 22 ? `:${conn.port}` : ""}</span>
                <span className="rdate">—</span>
              </button>
            ))}
          </div>
        ) : null}
      </div>
    </div>
  );
}
