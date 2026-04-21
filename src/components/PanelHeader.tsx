import type { ComponentType, ReactNode, SVGProps } from "react";

type LucideIcon = ComponentType<SVGProps<SVGSVGElement> & { size?: number | string }>;

type Props = {
  className?: string;
  icon?: LucideIcon;
  title: string;
  meta?: ReactNode;
  actions?: ReactNode;
};

export default function PanelHeader({ className = "", icon: Icon, title, meta, actions }: Props) {
  return (
    <div className={["panel-header", className].filter(Boolean).join(" ")}>
      <span className="ptitle">
        {Icon ? <Icon size={12} /> : null}
        {title}
      </span>
      <span className="pspacer" />
      {meta ? <span className="pmeta">{meta}</span> : null}
      {actions ? <span className="pactions">{actions}</span> : null}
    </div>
  );
}
