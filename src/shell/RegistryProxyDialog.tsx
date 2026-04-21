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
  { label: "DaoCloud (docker.m.daocloud.io)", value: "docker.m.daocloud.io" },
  { label: "阿里云 (registry.cn-hangzhou.aliyuncs.com)", value: "registry.cn-hangzhou.aliyuncs.com" },
  { label: "NJU (docker.nju.edu.cn)", value: "docker.nju.edu.cn" },
  { label: "USTC (docker.mirrors.ustc.edu.cn)", value: "docker.mirrors.ustc.edu.cn" },
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
              <label className="dlg-row-label">{t("Registry mirror")}</label>
              <input
                className="dlg-input"
                placeholder="docker.m.daocloud.io"
                value={m}
                onChange={(e) => setM(e.currentTarget.value)}
              />
              <div className="dlg-row-hint mono">
                {t("Prepended to pulls whose image ref does not already contain a registry (e.g. `nginx:latest` → `<mirror>/nginx:latest`).")}
              </div>
              <div className="dlg-chips">
                {MIRROR_PRESETS.map((pr) => (
                  <button
                    key={pr.value}
                    type="button"
                    className={"dlg-chip" + (m.trim() === pr.value ? " active" : "")}
                    onClick={() => setM(pr.value)}
                  >
                    {pr.label}
                  </button>
                ))}
                {m && (
                  <button type="button" className="dlg-chip" onClick={() => setM("")}>
                    {t("Clear")}
                  </button>
                )}
              </div>
            </div>

            <div className="dlg-row">
              <label className="dlg-row-label">{t("Pull proxy (HTTPS_PROXY)")}</label>
              <input
                className="dlg-input"
                placeholder="http://127.0.0.1:7890"
                value={p}
                onChange={(e) => setP(e.currentTarget.value)}
              />
              <div className="dlg-row-hint mono">
                {t("Applied only to this tab's `docker pull` as an env var. The remote daemon config is untouched.")}
              </div>
            </div>
          </div>
        </div>
        <div className="dlg-foot">
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
