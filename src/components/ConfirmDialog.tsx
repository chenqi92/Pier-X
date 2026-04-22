import { AlertTriangle, X } from "lucide-react";
import { useEffect } from "react";
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
  onConfirm,
  onCancel,
}: Props) {
  const { t } = useI18n();
  const { dialogStyle, handleProps } = useDraggableDialog(open);

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

  return (
    <div className="cmdp-overlay" onClick={onCancel}>
      <div
        className="dlg"
        style={{ ...dialogStyle, maxWidth: 420 }}
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
