import { ChevronDown, Plus, Slash } from "lucide-react";
import type { ReactNode } from "react";
import { useEffect, useState } from "react";

import { useI18n } from "../../i18n/useI18n";
import DbEnvTag from "./DbEnvTag";
import { DB_THEMES, inferEnv } from "./dbTheme";
import type { DbKind } from "../../lib/types";

export type DbHeaderInstance = {
  id: string;
  name: string;
  addr: string;
  via: string;
  lastUsed?: string | null;
  status: "up" | "down" | "unknown";
  /** Displayed under the name as a sub-line (engine + addr). */
  sub?: ReactNode;
};

type Props = {
  kind: DbKind;
  current: DbHeaderInstance;
  /** Other saved instances the user can swap to without leaving the panel. */
  others?: DbHeaderInstance[];
  onSwitch?: (id: string) => void;
  onAdd: () => void;
  onDisconnect: () => void;
};

/**
 * Compact "current connection" chip in the connected-panel header.
 * Clicking opens a dropdown with the other saved instances plus
 * Add / Disconnect actions. This is distinct from the full splash
 * picker — the splash is for cold starts, this is for hot-swapping.
 */
export default function DbHeaderPicker({
  kind,
  current,
  others = [],
  onSwitch,
  onAdd,
  onDisconnect,
}: Props) {
  const { t } = useI18n();
  const theme = DB_THEMES[kind];
  const { icon: Glyph } = theme;
  const [open, setOpen] = useState(false);

  // Close on Escape — matches the popover convention for the command
  // palette and other overlays in the shell.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open]);

  return (
    <div className="dbi">
      <button type="button" className="dbi-btn" onClick={() => setOpen((v) => !v)}>
        <span
          className="dbi-ic"
          style={{ color: theme.tintVar, background: theme.chipBgVar }}
        >
          <Glyph size={13} />
        </span>
        <div className="dbi-meta">
          <div className="dbi-name">
            <span>{current.name}</span>
            <DbEnvTag env={inferEnv(current.name)} />
          </div>
          <div className="dbi-sub">
            <span className={"db-status-dot " + (current.status === "up" ? "on" : "off")} />
            {current.sub ?? <>{current.addr}</>}
          </div>
        </div>
        <ChevronDown size={11} style={{ color: "var(--muted)", flex: "none" }} />
      </button>

      {open && (
        <>
          <div className="dbi-backdrop" onClick={() => setOpen(false)} />
          <div className="dbi-menu" role="menu">
            <div className="dbi-menu-head">
              <span>{t("Active connection")}</span>
              <span className="dbs-foot-spacer" />
            </div>
            <button
              type="button"
              className="dbi-menu-row sel"
              onClick={() => setOpen(false)}
            >
              <span className={"db-status-dot " + (current.status === "up" ? "on" : "off")} />
              <div className="dbi-menu-body">
                <div className="dbi-menu-name">
                  <span>{current.name}</span>
                  <DbEnvTag env={inferEnv(current.name)} />
                </div>
                <div className="dbi-menu-addr">
                  {current.addr}
                  {current.via ? (
                    <>
                      {" · "}
                      <span className="text-muted">{current.via}</span>
                    </>
                  ) : null}
                </div>
              </div>
              {current.lastUsed && (
                <span className="dbi-menu-stat">{current.lastUsed}</span>
              )}
            </button>

            {others.length > 0 && (
              <>
                <div className="dbi-menu-sep" />
                <div className="dbi-menu-head">
                  <span>{t("Switch to")}</span>
                </div>
                {others.map((ins) => (
                  <button
                    key={ins.id}
                    type="button"
                    className="dbi-menu-row"
                    onClick={() => {
                      onSwitch?.(ins.id);
                      setOpen(false);
                    }}
                  >
                    <span className={"db-status-dot " + (ins.status === "up" ? "on" : "off")} />
                    <div className="dbi-menu-body">
                      <div className="dbi-menu-name">
                        <span>{ins.name}</span>
                        <DbEnvTag env={inferEnv(ins.name)} />
                      </div>
                      <div className="dbi-menu-addr">
                        {ins.addr}
                        {ins.via ? (
                          <>
                            {" · "}
                            <span className="text-muted">{ins.via}</span>
                          </>
                        ) : null}
                      </div>
                    </div>
                    {ins.lastUsed && <span className="dbi-menu-stat">{ins.lastUsed}</span>}
                  </button>
                ))}
              </>
            )}

            <div className="dbi-menu-sep" />
            <button
              type="button"
              className="dbi-menu-row action"
              onClick={() => {
                onAdd();
                setOpen(false);
              }}
            >
              <Plus size={12} /> {t("Add connection")}
            </button>
            <button
              type="button"
              className="dbi-menu-row action"
              onClick={() => {
                onDisconnect();
                setOpen(false);
              }}
            >
              <Slash size={12} /> {t("Disconnect")}
            </button>
          </div>
        </>
      )}
    </div>
  );
}
