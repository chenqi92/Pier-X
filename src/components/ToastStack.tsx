import { AlertCircle, CheckCircle2, Info, TriangleAlert, X } from "lucide-react";
import { useToastStore, type ToastKind } from "../stores/useToastStore";

const KIND_META: Record<
  ToastKind,
  { icon: typeof Info; accent: string; bg: string }
> = {
  info: { icon: Info, accent: "var(--info)", bg: "var(--surface-2)" },
  success: { icon: CheckCircle2, accent: "var(--pos)", bg: "var(--pos-dim)" },
  warning: { icon: TriangleAlert, accent: "var(--warn)", bg: "var(--warn-dim)" },
  error: { icon: AlertCircle, accent: "var(--neg)", bg: "var(--neg-dim)" },
};

/**
 * Floating toast stack in the bottom-right. Mount once, at shell
 * scope; callers invoke notifications via `toast.info(...)` etc.
 * from `useToastStore` without needing a component reference.
 *
 * Each toast auto-dismisses after its duration (default 4s);
 * `kind: "error"` defaults to 6s since the user typically needs
 * more time to read a failure reason. Sticky toasts (duration = 0)
 * stay until explicitly closed.
 */
export default function ToastStack() {
  const toasts = useToastStore((s) => s.toasts);
  const dismiss = useToastStore((s) => s.dismiss);

  if (toasts.length === 0) return null;

  return (
    <div
      style={{
        position: "fixed",
        bottom: 28,
        right: 20,
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-2)",
        zIndex: 1000,
        pointerEvents: "none",
      }}
    >
      {toasts.map((t) => {
        const meta = KIND_META[t.kind];
        const Icon = meta.icon;
        return (
          <div
            key={t.id}
            role="status"
            style={{
              pointerEvents: "auto",
              display: "flex",
              alignItems: "flex-start",
              gap: "var(--sp-2)",
              minWidth: 280,
              maxWidth: 420,
              padding: "var(--sp-3) var(--sp-3)",
              background: meta.bg,
              border: `1px solid ${meta.accent}`,
              borderLeft: `3px solid ${meta.accent}`,
              borderRadius: "var(--radius-sm)",
              boxShadow: "var(--shadow-popover)",
              color: "var(--ink)",
              fontSize: "var(--ui-fs)",
            }}
          >
            <Icon size={14} color={meta.accent} style={{ marginTop: 2, flexShrink: 0 }} />
            <div style={{ flex: 1, lineHeight: 1.45, wordBreak: "break-word" }}>
              {t.message}
              {t.action && (
                <button
                  type="button"
                  className="mini-button"
                  style={{
                    marginLeft: "var(--sp-2)",
                    borderColor: meta.accent,
                    color: meta.accent,
                  }}
                  onClick={() => {
                    // Dismiss BEFORE running the callback so the
                    // toast disappears even if `onClick` opens a
                    // modal that traps focus and then errors out
                    // — better UX than leaving an orphan banner.
                    dismiss(t.id);
                    try {
                      t.action!.onClick();
                    } catch {
                      /* user-supplied callback */
                    }
                  }}
                >
                  {t.action.label}
                </button>
              )}
            </div>
            <button
              type="button"
              aria-label="Dismiss"
              onClick={() => dismiss(t.id)}
              style={{
                flexShrink: 0,
                background: "transparent",
                border: "none",
                cursor: "pointer",
                color: "var(--muted)",
                padding: 2,
                display: "flex",
                alignItems: "center",
              }}
            >
              <X size={12} />
            </button>
          </div>
        );
      })}
    </div>
  );
}
