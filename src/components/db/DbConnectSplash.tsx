import { Plus, Radio, RefreshCw, Star } from "lucide-react";
import type { CSSProperties, ReactNode } from "react";

import { useI18n } from "../../i18n/useI18n";
import DbSplashRow, { type DbSplashRowData } from "./DbSplashRow";
import { DB_THEMES } from "./dbTheme";
import type { DbKind } from "../../lib/types";

type ProbeState = "idle" | "scanning" | "error";

type Props = {
  kind: DbKind;
  /** "deploy@prod-api-01" — shown in the probe activity line. */
  probeTarget: string | null;
  probeState: ProbeState;
  onReprobe?: () => void;
  /** Auto-detected instances. Renders under "Auto-detected". */
  detected: DbSplashRowData[];
  /** Saved profiles. Renders under "Saved profiles". */
  saved: DbSplashRowData[];
  onAddManual: () => void;
  /** Displayed below the Add button — e.g. credentials-file path. */
  footerHint?: ReactNode;
  /** Explainer line under the title. */
  description?: ReactNode;
  /** Extra content rendered between the Saved list and the footer —
   *  used by SQLite / SFTP for kind-specific "open by path" forms. */
  extraBody?: ReactNode;
  /** When true, the default "Add connection manually…" button is
   *  hidden — callers using `extraBody` for the primary entrypoint
   *  can opt out of the duplicate action. */
  hideAddManual?: boolean;
};

const extraBodyStyle: CSSProperties = { marginTop: "var(--sp-3)" };

/**
 * Full-bleed splash that replaces the panel content when no DB
 * connection is active. Structured around three visual layers:
 *   1. header  — glyph + title + description
 *   2. probe   — "scan via {ssh-target}" line + re-probe action
 *   3. lists   — grouped Auto-detected / Saved rows
 *   4. footer  — manual-add entrypoint + credential hint
 */
export default function DbConnectSplash({
  kind,
  probeTarget,
  probeState,
  onReprobe,
  detected,
  saved,
  onAddManual,
  footerHint,
  description,
  extraBody,
  hideAddManual = false,
}: Props) {
  const { t } = useI18n();
  const theme = DB_THEMES[kind];
  const { icon: Glyph } = theme;

  const titleByKind: Record<DbKind, string> = {
    mysql: t("Not connected to a MySQL instance"),
    postgres: t("Not connected to a PostgreSQL instance"),
    redis: t("Not connected to a Redis instance"),
    sqlite: t("Not connected to a SQLite database"),
  };

  const defaultDescription: Record<DbKind, string> = {
    mysql: t("Pier-X probed this host over the SSH session for {daemon}. Pick a detected instance, a saved profile, or add one manually.", { daemon: theme.daemon }),
    postgres: t("Pier-X probed this host over the SSH session for {daemon}. Pick a detected instance, a saved profile, or add one manually.", { daemon: theme.daemon }),
    redis: t("Pier-X probed this host over the SSH session for {daemon}. Pick a detected instance, a saved profile, or add one manually.", { daemon: theme.daemon }),
    sqlite: t("Open a local database or pick one detected on the remote host."),
  };

  const probeDotClass =
    probeState === "scanning"
      ? "dbs-probe-dot scanning"
      : probeState === "error"
        ? "dbs-probe-dot off"
        : "dbs-probe-dot";

  return (
    <div className="dbs-splash">
      <div className="dbs-inner">
        <header className="dbs-head">
          <div
            className="dbs-glyph"
            style={{
              color: theme.tintVar,
              background: `color-mix(in srgb, ${theme.tintVar} 16%, transparent)`,
              borderColor: `color-mix(in srgb, ${theme.tintVar} 45%, transparent)`,
            }}
          >
            <Glyph size={22} />
          </div>
          <div>
            <div className="dbs-title">{titleByKind[kind]}</div>
            <div className="dbs-sub">{description ?? defaultDescription[kind]}</div>
          </div>
        </header>

        {probeTarget !== null && (
          <div className="dbs-probe">
            <span className={probeDotClass} />
            <span>
              {probeState === "scanning"
                ? t("Scanning via")
                : probeState === "error"
                  ? t("Probe failed via")
                  : t("Probe via")}{" "}
              <b>{probeTarget}</b>
            </span>
            <span className="sep">·</span>
            <span>{t("ss -tlnp · systemd units · /etc/{daemon}", { daemon: theme.daemon })}</span>
            <span className="dbs-probe-spacer" />
            {onReprobe && (
              <button
                className="btn is-ghost is-compact"
                type="button"
                disabled={probeState === "scanning"}
                onClick={onReprobe}
              >
                <RefreshCw size={10} /> {t("Re-probe")}
              </button>
            )}
          </div>
        )}

        {detected.length > 0 && (
          <>
            <div className="dbs-section">
              <span className="dbs-section-l">
                <Radio size={11} /> {t("Auto-detected on this host")}
              </span>
              <span className="dbs-section-r">
                {t("{count} found", { count: detected.length })}
              </span>
            </div>
            <div className="dbs-list">
              {detected.map((row) => (
                <DbSplashRow key={row.id} {...row} />
              ))}
            </div>
          </>
        )}

        {saved.length > 0 && (
          <>
            <div className="dbs-section">
              <span className="dbs-section-l">
                <Star size={11} /> {t("Saved profiles")}
              </span>
              <span className="dbs-section-r">{saved.length}</span>
            </div>
            <div className="dbs-list">
              {saved.map((row) => (
                <DbSplashRow key={row.id} {...row} />
              ))}
            </div>
          </>
        )}

        {detected.length === 0 && saved.length === 0 && !extraBody && (
          <div className="dbs-empty">{t("No saved or detected connections yet.")}</div>
        )}

        {extraBody && <div style={extraBodyStyle}>{extraBody}</div>}

        <div className="dbs-foot">
          {!hideAddManual && (
            <button className="btn is-primary is-compact" type="button" onClick={onAddManual}>
              <Plus size={11} /> {t("Add connection manually…")}
            </button>
          )}
          <span className="dbs-foot-spacer" />
          {footerHint && <span className="dbs-foot-hint">{footerHint}</span>}
        </div>
      </div>
    </div>
  );
}
