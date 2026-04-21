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
  onSettings,
  onCommandPalette,
  version,
  workspaceRoot,
}: Props) {
  const { t, locale } = useI18n();
  const { connections } = useConnectionStore();
  const recents = useRecentConnectionsStore((s) => s.recents);
  const [now, setNow] = useState(() => new Date());

  useEffect(() => {
    const id = window.setInterval(() => setNow(new Date()), 30_000);
    return () => window.clearInterval(id);
  }, []);

  const isMac = navigator.platform.includes("Mac");
  const mod = isMac ? "⌘" : "Ctrl+";
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const greeting = t(greetingFor(now.getHours()));
  const dateLine = formatDateLine(now, localeTag);
  const user = t("friend");
  const subtitle = `pier-x · ${version ? `${version} · ` : ""}${dateLine}`;

  const recentList = connections
    .map((c) => ({ conn: c, ts: recents[c.index] ?? 0 }))
    .filter((r) => r.ts > 0)
    .sort((a, b) => b.ts - a.ts)
    .slice(0, 12);

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
            <div className="welcome-recent-list">
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
                  <span className="rdate">{relativeTime(ts, localeTag)}</span>
                </button>
              ))}
            </div>
          </div>
        ) : connections.length > 0 ? (
          <div className="welcome-recent">
            <h4>{t("Saved servers")}</h4>
            <div className="welcome-recent-list">
              {connections.slice(0, 12).map((conn) => (
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
          </div>
        ) : null}
      </div>
    </div>
  );
}
