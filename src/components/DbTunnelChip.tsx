import { useEffect, useRef, useState } from "react";
import { ChevronDown, RefreshCw, X } from "lucide-react";
import { useI18n } from "../i18n/useI18n";
import Badge from "./Badge";

type Props = {
  /** Tunnel local port, `null` when the tunnel is not open. */
  localPort: number | null;
  /** True while a tunnel open / refresh / close is in-flight. */
  busy: boolean;
  /** True when a tunnel operation (not the query) failed. */
  hasError?: boolean;
  /** Rebuild the tunnel — closes and reopens with the same slot. */
  onRebuild: () => void;
  /** Close the tunnel. */
  onClose: () => void;
};

/**
 * Single-chip replacement for the old "Open / Refresh / Close Tunnel"
 * button row. Shows the live local port (or a state label), and on
 * click opens a mini menu with Rebuild / Stop.
 *
 * The chip is intentionally non-interactive until the tunnel exists —
 * opening happens as a side effect of Connect, not as a user action.
 */
export default function DbTunnelChip({
  localPort,
  busy,
  hasError,
  onRebuild,
  onClose,
}: Props) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const onDocClick = (e: MouseEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("mousedown", onDocClick);
    return () => window.removeEventListener("mousedown", onDocClick);
  }, [open]);

  const tone = hasError ? "neg" : localPort ? "info" : "muted";
  const label = busy
    ? t("SSH tunnel · opening…")
    : hasError
      ? t("SSH tunnel · error")
      : localPort
        ? t("SSH tunnel · 127.0.0.1:{port}", { port: localPort })
        : t("SSH tunnel · idle");
  const interactive = !busy && !!localPort;

  return (
    <div className="db-tunnel-chip" ref={rootRef}>
      <button
        className="db-tunnel-chip__trigger"
        disabled={!interactive}
        onClick={() => setOpen((v) => !v)}
        title={
          interactive
            ? t("Tunnel actions")
            : t("The tunnel opens automatically when you connect.")
        }
        type="button"
      >
        <Badge tone={tone}>{label}</Badge>
        {interactive && <ChevronDown size={10} />}
      </button>
      {open && (
        <div className="db-tunnel-chip__menu" role="menu">
          <button
            className="db-tunnel-chip__item"
            onClick={() => {
              setOpen(false);
              onRebuild();
            }}
            role="menuitem"
            type="button"
          >
            <RefreshCw size={11} /> {t("Rebuild tunnel")}
          </button>
          <button
            className="db-tunnel-chip__item db-tunnel-chip__item--danger"
            onClick={() => {
              setOpen(false);
              onClose();
            }}
            role="menuitem"
            type="button"
          >
            <X size={11} /> {t("Stop tunnel")}
          </button>
        </div>
      )}
    </div>
  );
}
