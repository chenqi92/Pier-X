import type { ReactNode } from "react";

type Props = {
  tinted?: boolean;
  children: ReactNode;
  className?: string;
};

export default function Pill({ tinted, children, className }: Props) {
  const cls = `pill${tinted ? " is-tinted" : ""}${className ? ` ${className}` : ""}`;
  return <span className={cls}>{children}</span>;
}
