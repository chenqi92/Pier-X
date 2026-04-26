import { Container, Key, Link2, Loader2, Play, Server } from "lucide-react";
import type { ReactNode } from "react";

import DbEnvTag from "./DbEnvTag";
import type { DbEnv } from "./dbTheme";

/** One row of the splash's detected / saved list. */
export type DbSplashRowData = {
  id: string;
  name: string;
  env: DbEnv;
  engine: string;
  addr: string;
  via: {
    kind: "tunnel" | "remote" | "local" | "direct";
    label: string;
  };
  user?: string;
  authHint?: string;
  stats: ReactNode;
  lastUsed?: string | null;
  status: "up" | "down" | "unknown";
  /** One of the `--svc-*` CSS color expressions. Used to tint the Connect button border. */
  tintVar: string;
  connectLabel: string;
  onConnect: () => void;
  /** True while the user's click is in flight — swap the Play glyph for a
   *  spinner and disable the row so the click clearly registered. */
  pending?: boolean;
};

function ViaIcon({ kind }: { kind: DbSplashRowData["via"]["kind"] }) {
  switch (kind) {
    case "tunnel":
      return <Link2 size={9} />;
    case "remote":
      return <Server size={9} />;
    case "local":
      return <Container size={9} />;
    default:
      return <Server size={9} />;
  }
}

export default function DbSplashRow({
  name,
  env,
  engine,
  addr,
  via,
  user,
  authHint,
  stats,
  lastUsed,
  status,
  tintVar,
  connectLabel,
  onConnect,
  pending = false,
}: DbSplashRowData) {
  return (
    <button
      type="button"
      className="dbs-row"
      onClick={onConnect}
      disabled={pending}
      aria-busy={pending || undefined}
    >
      <span className={"db-status-dot " + (status === "up" ? "on" : "off")} />
      <div className="dbs-row-main">
        <div className="dbs-row-name">
          <span>{name}</span>
          <DbEnvTag env={env} />
        </div>
        <div className="dbs-row-meta">
          <span>{engine}</span>
          <span className="sep">·</span>
          <span>{addr}</span>
        </div>
        <div className="dbs-row-auth">
          <span className="dbs-via">
            <ViaIcon kind={via.kind} />
            {via.label}
          </span>
          {user ? (
            <>
              <span className="sep">·</span>
              <span>
                <Key size={9} /> {user}
              </span>
            </>
          ) : null}
          {authHint ? (
            <>
              <span className="sep">·</span>
              <span className="dbs-auth-from">{authHint}</span>
            </>
          ) : null}
        </div>
      </div>
      <div className="dbs-row-stats">{stats}</div>
      <span className="dbs-last">{lastUsed ?? "—"}</span>
      <span
        className="btn is-ghost is-compact dbs-connect"
        style={{
          color: tintVar,
          borderColor: `color-mix(in srgb, ${tintVar} 50%, var(--line-2))`,
        }}
      >
        {pending ? (
          <Loader2 size={10} className="dbs-spin" />
        ) : (
          <Play size={10} />
        )}{" "}
        {connectLabel}
      </span>
    </button>
  );
}
