import type { ComponentType, ReactNode, SVGProps } from "react";

type LucideIcon = ComponentType<SVGProps<SVGSVGElement> & { size?: number | string }>;

type Props = {
  icon: LucideIcon;
  /** Tint token for the icon chip background (CSS variable or color) */
  tint?: string;
  /** Tint token for the icon itself */
  iconTint?: string;
  name: ReactNode;
  sub?: ReactNode;
  tag?: ReactNode;
};

export default function DbConnRow({ icon: Icon, tint, iconTint, name, sub, tag }: Props) {
  const iconStyle: React.CSSProperties = {};
  if (tint) iconStyle.background = tint;
  if (iconTint) iconStyle.color = iconTint;

  return (
    <div className="db-conn-row">
      <span className="db-conn-icon" style={iconStyle}>
        <Icon size={12} />
      </span>
      <div className="db-conn-title">
        <div className="db-conn-name">{name}</div>
        {sub ? <div className="db-conn-sub mono">{sub}</div> : null}
      </div>
      {tag ? <span className="db-conn-tag mono">{tag}</span> : null}
    </div>
  );
}
