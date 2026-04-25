import { X } from "lucide-react";
import { useEffect, useRef } from "react";
import type { ReactNode } from "react";
import { createPortal } from "react-dom";
import IconButton from "./IconButton";
import { useDraggableDialog } from "./useDraggableDialog";
import { useI18n } from "../i18n/useI18n";

type Size = "sm" | "md" | "lg" | "xl";

type Props = {
  open: boolean;
  title: ReactNode;
  subtitle?: ReactNode;
  size?: Size;
  tall?: boolean;
  closeOnOverlay?: boolean;
  closeOnEscape?: boolean;
  initialFocusRef?: React.RefObject<HTMLElement | null>;
  onClose: () => void;
  footer?: ReactNode;
  children: ReactNode;
};

const SIZE_CLASS: Record<Size, string> = {
  sm: "dlg--size-sm",
  md: "dlg--size-md",
  lg: "dlg--size-lg",
  xl: "dlg--size-xl",
};

export default function Dialog({
  open,
  title,
  subtitle,
  size = "md",
  tall = false,
  closeOnOverlay = true,
  closeOnEscape = true,
  initialFocusRef,
  onClose,
  footer,
  children,
}: Props) {
  const { t } = useI18n();
  const { dialogStyle, handleProps } = useDraggableDialog(open);
  const bodyRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open || !closeOnEscape) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, closeOnEscape, onClose]);

  useEffect(() => {
    if (!open) return;
    const focusTarget =
      initialFocusRef?.current ??
      bodyRef.current?.querySelector<HTMLElement>(
        "input:not([disabled]):not([type='hidden']), textarea:not([disabled]), [data-autofocus]",
      );
    focusTarget?.focus({ preventScroll: true });
  }, [open, initialFocusRef]);

  if (!open) return null;

  const className = [
    "dlg",
    SIZE_CLASS[size],
    tall ? "dlg--tall" : "",
  ]
    .filter(Boolean)
    .join(" ");

  return createPortal(
    <div
      className="dlg-overlay"
      onMouseDown={(event) => {
        if (closeOnOverlay && event.target === event.currentTarget) onClose();
      }}
    >
      <div className={className} style={dialogStyle} onMouseDown={(e) => e.stopPropagation()}>
        <div className="dlg-head" {...handleProps}>
          <div className="dlg-head__copy">
            <div className="dlg-head__title">{title}</div>
            {subtitle ? <div className="dlg-head__subtitle">{subtitle}</div> : null}
          </div>
          <div style={{ flex: 1 }} />
          <IconButton variant="mini" onClick={onClose} title={t("Close")}>
            <X size={12} />
          </IconButton>
        </div>
        <div ref={bodyRef} className="dlg-body dlg-body--plain">
          {children}
        </div>
        {footer ? <div className="dlg-foot">{footer}</div> : null}
      </div>
    </div>,
    document.body,
  );
}
