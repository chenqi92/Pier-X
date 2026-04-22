import { Settings2, X } from "lucide-react";
import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import IconButton from "../components/IconButton";
import { useDraggableDialog } from "../components/useDraggableDialog";
import { useI18n } from "../i18n/useI18n";

type Props = {
  open: boolean;
  mirror: string;
  proxy: string;
  onClose: () => void;
  onSave: (mirror: string, proxy: string) => void;
};

const MIRROR_PRESETS: Array<{ label: string; value: string }> = [
  { label: "DaoCloud", value: "docker.m.daocloud.io" },
  { label: "阿里云", value: "registry.cn-hangzhou.aliyuncs.com" },
  { label: "NJU", value: "docker.nju.edu.cn" },
  { label: "USTC", value: "docker.mirrors.ustc.edu.cn" },
];

export default function RegistryProxyDialog({ open, mirror, proxy, onClose, onSave }: Props) {
  const { t } = useI18n();
  const { dialogStyle, handleProps } = useDraggableDialog(open);
  const [m, setM] = useState(mirror);
  const [p, setP] = useState(proxy);

  useEffect(() => {
    if (open) {
      setM(mirror);
      setP(proxy);
    }
  }, [open, mirror, proxy]);

  if (!open) return null;

  // Stack input + preset buttons + hint inside the grid's 1fr column so
  // they don't bleed into the label column via auto-flow. Matches the
  // layout NewConnectionDialog uses for its host/port row.
  const stackStyle: React.CSSProperties = {
    display: "flex",
    flexDirection: "column",
    gap: "var(--sp-2)",
    minWidth: 0,
  };
  // The stack is taller than a single input, so pin the row label to the
  // top of the row (nudging down a hair to line up with the first line of
  // the input) instead of letting `.dlg-row { align-items: center }`
  // float it to the middle of the stack.
  const labelStyle: React.CSSProperties = {
    alignSelf: "start",
    paddingTop: "var(--sp-1-5)",
  };

  return createPortal(
    <div className="cmdp-overlay" onClick={onClose}>
      <div className="dlg dlg--proxy" style={dialogStyle} onClick={(e) => e.stopPropagation()}>
        <div className="dlg-head" {...handleProps}>
          <span className="dlg-title">
            <Settings2 size={12} />
            {t("Registry / proxy settings")}
          </span>
          <div style={{ flex: 1 }} />
          <IconButton variant="mini" onClick={onClose} title={t("Close")}>
            <X size={12} />
          </IconButton>
        </div>
        <div className="dlg-body dlg-body--form">
          <div className="dlg-form">
            <div className="dlg-row">
              <label className="dlg-row-label" style={labelStyle}>{t("Registry mirror")}</label>
              <div style={stackStyle}>
                <input
                  className="dlg-input mono"
                  placeholder="docker.m.daocloud.io"
                  value={m}
                  onChange={(e) => setM(e.currentTarget.value)}
                />
                <div className="dlg-opts" role="radiogroup" aria-label={t("Mirror presets")}>
                  {MIRROR_PRESETS.map((pr) => (
                    <button
                      key={pr.value}
                      type="button"
                      role="radio"
                      aria-checked={m.trim() === pr.value}
                      className={"dlg-opt" + (m.trim() === pr.value ? " active" : "")}
                      title={pr.value}
                      onClick={() => setM(pr.value)}
                    >
                      {pr.label}
                    </button>
                  ))}
                  {m && (
                    <button
                      type="button"
                      className="dlg-opt"
                      onClick={() => setM("")}
                    >
                      {t("Clear")}
                    </button>
                  )}
                </div>
                <div className="dlg-row-hint">
                  {t("Prepended to pulls whose image ref does not already contain a registry (e.g. nginx:latest → <mirror>/nginx:latest).")}
                </div>
              </div>
            </div>

            <div className="dlg-row">
              <label className="dlg-row-label" style={labelStyle}>{t("Pull proxy (HTTPS_PROXY)")}</label>
              <div style={stackStyle}>
                <input
                  className="dlg-input mono"
                  placeholder="http://127.0.0.1:7890"
                  value={p}
                  onChange={(e) => setP(e.currentTarget.value)}
                />
                <div className="dlg-row-hint">
                  {t("Applied only to this tab's docker pull as an env var. The remote daemon config is untouched.")}
                </div>
              </div>
            </div>
          </div>
        </div>
        <div className="dlg-foot">
          <div style={{ flex: 1 }} />
          <button type="button" className="gb-btn" onClick={onClose}>{t("Cancel")}</button>
          <button type="button" className="gb-btn primary" onClick={() => onSave(m, p)}>
            {t("Save")}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
