import { Plus } from "lucide-react";
import type { CSSProperties, ReactNode } from "react";
import { useMemo } from "react";
import { useConnectionStore } from "../stores/useConnectionStore";
import { useRecentConnectionsStore } from "../stores/useRecentConnectionsStore";
import { useI18n } from "../i18n/useI18n";

type Props = {
  icon: ReactNode;
  title: string;
  subtitle: string;
  /** CSS color expression used to tint the icon badge (defaults to accent). */
  tintVar?: string;
  /** Tag label to stamp on each recent row (e.g. "ssh", "mysql"). */
  tagLabel?: string;
  onConnectSaved: (index: number) => void;
  onNewConnection: () => void;
  /** Small hint shown beside the "New connection" button. */
  footerHint?: ReactNode;
  /** Heading above the recent list. */
  recentLabel?: string;
};

export default function ConnectSplash({
  icon,
  title,
  subtitle,
  tintVar = "var(--accent)",
  tagLabel,
  onConnectSaved,
  onNewConnection,
  footerHint,
  recentLabel,
}: Props) {
  const { t } = useI18n();
  const connections = useConnectionStore((s) => s.connections);
  const recents = useRecentConnectionsStore((s) => s.recents);

  const rows = useMemo(() => {
    const withTs = connections.map((c) => ({ conn: c, ts: recents[c.index] ?? 0 }));
    const recentsFirst = withTs
      .filter((r) => r.ts > 0)
      .sort((a, b) => b.ts - a.ts);
    const rest = withTs
      .filter((r) => r.ts === 0)
      .sort((a, b) => a.conn.index - b.conn.index);
    return [...recentsFirst, ...rest].slice(0, 5);
  }, [connections, recents]);

  const iconStyle: CSSProperties = {
    background: `color-mix(in srgb, ${tintVar} 22%, transparent)`,
    color: tintVar,
  };

  return (
    <div className="cs-splash">
      <div className="cs-card">
        <div className="cs-icon" style={iconStyle}>{icon}</div>
        <div className="cs-title">{title}</div>
        <div className="cs-sub">{subtitle}</div>

        {rows.length > 0 && (
          <div className="cs-list">
            <div className="cs-list-head mono">{recentLabel ?? t("Recent connections")}</div>
            {rows.map(({ conn, ts }) => (
              <button
                key={conn.index}
                type="button"
                className="cs-row"
                onClick={() => onConnectSaved(conn.index)}
              >
                <span className={"cs-dot " + (ts > 0 ? "on" : "off")} />
                <div className="cs-main">
                  <div className="cs-name">{conn.name || `${conn.user}@${conn.host}`}</div>
                  <div className="cs-meta mono">
                    {conn.user}@{conn.host}{conn.port !== 22 ? `:${conn.port}` : ""}
                  </div>
                </div>
                {tagLabel ? <span className="cs-tag mono">{tagLabel}</span> : null}
              </button>
            ))}
          </div>
        )}

        <div className="cs-foot">
          <button className="btn is-ghost is-compact" type="button" onClick={onNewConnection}>
            <Plus size={11} /> {t("New connection…")}
          </button>
          <div style={{ flex: 1 }} />
          {footerHint ? <span className="cs-foot-hint mono">{footerHint}</span> : null}
        </div>
      </div>
    </div>
  );
}
