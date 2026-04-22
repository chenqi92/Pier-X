import { FilePlus2, FolderPlus, X } from "lucide-react";
import { useEffect, useRef } from "react";
import IconButton from "./IconButton";
import { useDraggableDialog } from "./useDraggableDialog";
import { useI18n } from "../i18n/useI18n";

type Props = {
  open: boolean;
  /** Which option is currently selected. The caller owns this state
   *  so the dialog preserves the user's last choice across re-opens. */
  kind: "file" | "dir";
  name: string;
  /** Absolute remote path of the directory the new entry will be
   *  created in. Rendered under the name input so the user knows
   *  where it will land (especially relevant when the dialog opens
   *  from a nested right-click). */
  parentPath: string;
  busy?: boolean;
  onKindChange: (kind: "file" | "dir") => void;
  onNameChange: (name: string) => void;
  onSubmit: () => void;
  onClose: () => void;
};

/** New-file-or-folder dialog for the SFTP panel. Replaces the two
 *  separate inline "quickrow" editors (mkdir / touch) with one
 *  consistent surface that matches the other Pier-X dialogs. */
export default function SftpNewEntryDialog({
  open,
  kind,
  name,
  parentPath,
  busy,
  onKindChange,
  onNameChange,
  onSubmit,
  onClose,
}: Props) {
  const { t } = useI18n();
  const { dialogStyle, handleProps } = useDraggableDialog(open);
  const inputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const el = inputRef.current;
    if (el) {
      el.focus();
      el.select();
    }
  }, [open]);

  useEffect(() => {
    if (!open) return;
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape" && !busy) onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, busy, onClose]);

  if (!open) return null;

  const canSubmit = !busy && name.trim().length > 0;
  const Icon = kind === "dir" ? FolderPlus : FilePlus2;

  return (
    <div className="dlg-overlay" onClick={onClose}>
      <div
        className="dlg dlg--new-entry"
        style={dialogStyle}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="dlg-head" {...handleProps}>
          <span className="dlg-title">
            <Icon size={13} />
            {t("New item")}
          </span>
          <div style={{ flex: 1 }} />
          <IconButton variant="mini" onClick={onClose} title={t("Close")}>
            <X size={12} />
          </IconButton>
        </div>

        <div className="dlg-body dlg-body--form">
          <div className="dlg-form">
            <div className="dlg-row">
              <label className="dlg-row-label">{t("Kind")}</label>
              <div className="dlg-opts" role="radiogroup" aria-label={t("Kind")}>
                <button
                  type="button"
                  role="radio"
                  aria-checked={kind === "file"}
                  className={"dlg-opt" + (kind === "file" ? " active" : "")}
                  onClick={() => onKindChange("file")}
                  disabled={busy}
                >
                  {t("File")}
                </button>
                <button
                  type="button"
                  role="radio"
                  aria-checked={kind === "dir"}
                  className={"dlg-opt" + (kind === "dir" ? " active" : "")}
                  onClick={() => onKindChange("dir")}
                  disabled={busy}
                >
                  {t("Folder")}
                </button>
              </div>
            </div>
            <div className="dlg-row">
              <label className="dlg-row-label">{t("Name")}</label>
              <input
                ref={inputRef}
                className="dlg-input mono"
                value={name}
                onChange={(e) => onNameChange(e.currentTarget.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && canSubmit) {
                    e.preventDefault();
                    onSubmit();
                  }
                }}
                placeholder={kind === "dir" ? t("logs") : t("config.conf")}
                disabled={busy}
                spellCheck={false}
              />
            </div>
            <div className="dlg-row">
              <label className="dlg-row-label">{t("In")}</label>
              <span className="dlg-row-hint mono" title={parentPath}>{parentPath || "/"}</span>
            </div>
          </div>
        </div>

        <div className="dlg-foot">
          <div style={{ flex: 1 }} />
          <button type="button" className="gb-btn" onClick={onClose} disabled={busy}>
            {t("Cancel")}
          </button>
          <button
            type="button"
            className="gb-btn primary"
            onClick={onSubmit}
            disabled={!canSubmit}
          >
            {t("Create")}
          </button>
        </div>
      </div>
    </div>
  );
}
