import { X } from "lucide-react";
import type { ReactNode } from "react";
import { useI18n } from "../i18n/useI18n";

type Tone = "info" | "error";
/** `note` → the banner-style `.lg-note` used by panel headers.
 *  `status` → the inline `.status-note` used inside forms/dialogs. */
type Variant = "note" | "status";

type Props = {
  tone?: Tone;
  variant?: Variant;
  className?: string;
  onDismiss: () => void;
  /** Optional control rendered between the body and the close button —
   *  e.g. a "retry" / "elevate" action tied to the note's content. */
  action?: ReactNode;
  children: ReactNode;
};

export default function DismissibleNote({
  tone = "info",
  variant = "note",
  className,
  onDismiss,
  action,
  children,
}: Props) {
  const { t } = useI18n();
  const baseClass =
    variant === "status"
      ? "status-note" + (tone === "error" ? " status-note--error" : "")
      : "lg-note" + (tone === "error" ? " lg-note--error" : "");
  return (
    <div className={baseClass + " lg-note--dismissible" + (className ? ` ${className}` : "")}>
      <div className="lg-note-body">{children}</div>
      {action && <div className="lg-note-action">{action}</div>}
      <button
        type="button"
        className="lg-note-close"
        onClick={onDismiss}
        title={t("Dismiss")}
        aria-label={t("Dismiss")}
      >
        <X size={11} />
      </button>
    </div>
  );
}
