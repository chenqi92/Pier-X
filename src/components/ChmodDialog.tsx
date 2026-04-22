import { useEffect, useMemo, useState } from "react";
import { Lock, X } from "lucide-react";
import IconButton from "./IconButton";
import { useDraggableDialog } from "./useDraggableDialog";
import { useI18n } from "../i18n/useI18n";
import { modeToSymbolic } from "../lib/sftpEditor";

type Props = {
  open: boolean;
  path: string;
  /** Current octal permissions (low 12 bits). Null when the server
   *  didn't report permissions — we default the UI to 644. */
  initialMode: number | null;
  onSubmit: (mode: number) => Promise<void> | void;
  onClose: () => void;
  busy?: boolean;
};

type Triad = { r: boolean; w: boolean; x: boolean };

function triadFromBits(b: number): Triad {
  return { r: !!(b & 4), w: !!(b & 2), x: !!(b & 1) };
}

function triadToBits(t: Triad): number {
  return (t.r ? 4 : 0) | (t.w ? 2 : 0) | (t.x ? 1 : 0);
}

function modeToTriads(mode: number): { owner: Triad; group: Triad; other: Triad } {
  const m = mode & 0o777;
  return {
    owner: triadFromBits((m >> 6) & 7),
    group: triadFromBits((m >> 3) & 7),
    other: triadFromBits(m & 7),
  };
}

function triadsToMode(t: { owner: Triad; group: Triad; other: Triad }): number {
  return (triadToBits(t.owner) << 6) | (triadToBits(t.group) << 3) | triadToBits(t.other);
}

/** Permission editor shared by SFTP (and eventually any panel that
 *  lands a remote chmod surface). Single source of truth is the
 *  octal number in state — checkboxes derive from it and call back
 *  with an updated octal. */
export default function ChmodDialog({ open, path, initialMode, onSubmit, onClose, busy }: Props) {
  const { t } = useI18n();
  const { dialogStyle, handleProps } = useDraggableDialog(open);
  const [mode, setMode] = useState<number>(initialMode ?? 0o644);
  const [octalDraft, setOctalDraft] = useState<string>(
    (initialMode ?? 0o644).toString(8).padStart(3, "0"),
  );

  useEffect(() => {
    if (open) {
      const m = (initialMode ?? 0o644) & 0o777;
      setMode(m);
      setOctalDraft(m.toString(8).padStart(3, "0"));
    }
  }, [open, initialMode]);

  const triads = useMemo(() => modeToTriads(mode), [mode]);

  const setTriad = (scope: "owner" | "group" | "other", key: "r" | "w" | "x", value: boolean) => {
    const next = { ...triads, [scope]: { ...triads[scope], [key]: value } };
    const nm = triadsToMode(next);
    setMode(nm);
    setOctalDraft(nm.toString(8).padStart(3, "0"));
  };

  const onOctalChange = (raw: string) => {
    const cleaned = raw.replace(/[^0-7]/g, "").slice(0, 4);
    setOctalDraft(cleaned);
    if (/^[0-7]{3,4}$/.test(cleaned)) {
      setMode(parseInt(cleaned, 8) & 0o777);
    }
  };

  const submit = async () => {
    if (busy) return;
    await onSubmit(mode & 0o777);
  };

  if (!open) return null;

  const symbolic = modeToSymbolic(mode);

  return (
    <div className="dlg-overlay" onClick={onClose}>
      <div
        className="dlg dlg--chmod"
        style={dialogStyle}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="dlg-head" {...handleProps}>
          <span className="dlg-title">
            <Lock size={13} />
            {t("Change permissions")}
          </span>
          <div style={{ flex: 1 }} />
          <IconButton variant="mini" onClick={onClose} title={t("Close")}>
            <X size={12} />
          </IconButton>
        </div>

        <div className="dlg-body dlg-body--form">
          <div className="chmod-path mono" title={path}>{path}</div>

          <div className="chmod-grid">
            <div className="chmod-row chmod-row--head">
              <span />
              <span>{t("Read")}</span>
              <span>{t("Write")}</span>
              <span>{t("Execute")}</span>
            </div>
            {(["owner", "group", "other"] as const).map((scope) => (
              <div key={scope} className="chmod-row">
                <span className="chmod-scope">{t(scope === "owner" ? "Owner" : scope === "group" ? "Group" : "Other")}</span>
                {(["r", "w", "x"] as const).map((bit) => (
                  <label key={bit} className="chmod-cell">
                    <input
                      type="checkbox"
                      checked={triads[scope][bit]}
                      onChange={(e) => setTriad(scope, bit, e.currentTarget.checked)}
                      disabled={busy}
                    />
                  </label>
                ))}
              </div>
            ))}
          </div>

          <div className="chmod-octal-row">
            <label className="chmod-octal-label mono">{t("Octal")}</label>
            <input
              className="field-input field-input--compact mono chmod-octal-input"
              value={octalDraft}
              onChange={(e) => onOctalChange(e.currentTarget.value)}
              onKeyDown={(e) => { if (e.key === "Enter") void submit(); }}
              maxLength={4}
              spellCheck={false}
              disabled={busy}
            />
            <span className="chmod-symbolic mono">{symbolic}</span>
          </div>

          <div className="chmod-actions">
            <button
              type="button"
              className="btn is-primary is-compact"
              onClick={() => void submit()}
              disabled={busy}
            >
              {t("Apply")}
            </button>
            <button
              type="button"
              className="btn is-ghost is-compact"
              onClick={onClose}
              disabled={busy}
            >
              {t("Cancel")}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
