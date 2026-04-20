import type { ButtonHTMLAttributes, ReactNode } from "react";

type Variant = "mini" | "icon" | "tool";

type Props = ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: Variant;
  active?: boolean;
  destructive?: boolean;
  dim?: boolean;
  children: ReactNode;
};

const CLASS_BY_VARIANT: Record<Variant, string> = {
  mini: "mini-btn",
  icon: "icon-btn",
  tool: "ts-btn",
};

export default function IconButton({
  variant = "icon",
  active,
  destructive,
  dim,
  className,
  children,
  ...rest
}: Props) {
  const classes = [
    CLASS_BY_VARIANT[variant],
    active ? "is-active" : "",
    destructive ? "is-destructive" : "",
    dim ? "is-dim" : "",
    className ?? "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <button type="button" className={classes} {...rest}>
      {children}
    </button>
  );
}
