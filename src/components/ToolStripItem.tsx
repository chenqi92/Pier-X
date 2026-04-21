import type { ComponentType, SVGProps } from "react";
import { useI18n } from "../i18n/useI18n";

type LucideIcon = ComponentType<SVGProps<SVGSVGElement> & { size?: number | string }>;

type Props = {
  icon: LucideIcon;
  label: string;
  active?: boolean;
  detected?: boolean;
  dim?: boolean;
  onClick?: () => void;
};

export default function ToolStripItem({
  icon: Icon,
  label,
  active,
  detected,
  dim,
  onClick,
}: Props) {
  const { t } = useI18n();
  const cls = ["ts-btn", active ? "is-active" : "", dim ? "is-dim" : ""]
    .filter(Boolean)
    .join(" ");
  const title = dim ? `${label} · ${t("not detected on this host")}` : label;
  return (
    <button type="button" className={cls} title={title} onClick={onClick}>
      <Icon size={16} />
      {detected ? <span className="ts-det" /> : null}
    </button>
  );
}
