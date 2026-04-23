import { AlertTriangle, X } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { CSSProperties } from "react";
import IconButton from "./IconButton";
import { useDraggableDialog } from "./useDraggableDialog";
import { useI18n } from "../i18n/useI18n";

type Tone = "neutral" | "destructive";

type Props = {
  open: boolean;
  title: string;
  /** Body text. Interpolated with the same `t()` vars the caller
   *  passes — kept simple on purpose, no rich content. */
  message: string;
  /** Label for the confirm button. Defaults to "Confirm" /
   *  "Delete" based on tone. */
  confirmLabel?: string;
  cancelLabel?: string;
  /** `destructive` paints the confirm button red and swaps the
   *  icon to a warning triangle. */
  tone?: Tone;
  /** Optional viewport-coord anchor. When provided, the dialog is
   *  positioned near these coordinates (clamped to the viewport)
   *  instead of centered in the overlay — used so the confirm prompt
   *  appears near the cursor that triggered a context-menu "Delete". */
  anchor?: { x: number; y: number };
  onConfirm: () => void;
  onCancel: () => void;
};

/**
 * Minimal themed confirmation dialog — used in place of
 * `window.confirm` so the shell's theme tokens apply and the
 * prompt doesn't block the entire OS event loop. Mirrors the
 * chrome of `NewConnectionDialog` (`cmdp-overlay` + `dlg`).
 */
export default function ConfirmDialog({
  open,
  title,
  message,
  confirmLabel,
  cancelLabel,
  tone = "neutral",
  anchor,
  onConfirm,
  onCancel,
}: Props) {
  const { t } = useI18n();
  const { dialogStyle, handleProps } = useDraggableDialog(open);
  const dialogRef = useRef<HTMLDivElement>(null);
  const [anchorPos, setAnchorPos] = useState<{ left: number; top: number } | null>(null);

  // Clamp the anchor to the viewport once we know the dialog's
  // rendered size. Falls back to a guess on the first paint so the
  // dialog doesn't flash in the top-left; the layout effect then
  // corrects it on the same frame.
  useLayoutEffect(() => {
    if (!open || !anchor) {
      setAnchorPos(null);
      return;
    }
    const el = dialogRef.current;
    const w = el?.offsetWidth ?? 420;
    const h = el?.offsetHeight ?? 190;
    const margin = 8;
    const left = Math.min(
      Math.max(anchor.x + 8, margin),
      Math.max(margin, window.innerWidth - w - margin),
    );
    const top = Math.min(
      Math.max(anchor.y + 8, margin),
      Math.max(margin, window.innerHeight - h - margin),
    );
    setAnchorPos({ left, top });
  }, [open, anchor]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        onCancel();
      } else if (e.key === "Enter") {
        onConfirm();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onCancel, onConfirm]);

  if (!open) return null;

  const effectiveConfirm =
    confirmLabel ?? (tone === "destructive" ? t("Delete") : t("Confirm"));
  const effectiveCancel = cancelLabel ?? t("Cancel");

  const positionStyle: CSSProperties = anchor
    ? {
        position: "absolute",
        left: anchorPos?.left ?? anchor.x,
        top: anchorPos?.top ?? anchor.y,
        margin: 0,
        // Hide until the layout effect clamps to the viewport to avoid
        // a one-frame flash in the corner when the anchor starts near
        // the right/bottom edge.
        visibility: anchorPos ? "visible" : "hidden",
      }
    : {};

  return (
    <div
      className={"cmdp-overlay" + (anchor ? " cmdp-overlay--anchored" : "")}
      onClick={onCancel}
    >
      <div
        ref={dialogRef}
        className="dlg"
        style={{ ...dialogStyle, ...positionStyle, maxWidth: 420 }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="dlg-head" {...handleProps}>
          <span className="dlg-title">
            <AlertTriangle
              size={13}
              style={{ color: tone === "destructive" ? "var(--neg)" : "var(--warn)" }}
            />
            {title}
          </span>
          <div style={{ flex: 1 }} />
          <IconButton variant="mini" onClick={onCancel} title={t("Close")}>
            <X size={12} />
          </IconButton>
        </div>
        <div className="dlg-body dlg-body--form">
          <div className="dlg-form">
            <div className="status-note" style={{ whiteSpace: "pre-wrap" }}>
              {message}
            </div>
          </div>
        </div>
        <div className="dlg-foot">
          <div style={{ flex: 1 }} />
          <button className="gb-btn" onClick={onCancel} type="button">
            {effectiveCancel}
          </button>
          <button
            className={
              tone === "destructive" ? "gb-btn gb-btn--destructive" : "gb-btn"
            }
            onClick={onConfirm}
            type="button"
          >
            {effectiveConfirm}
          </button>
        </div>
      </div>
    </div>
  );
}
