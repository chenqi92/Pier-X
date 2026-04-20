import type { ReactNode } from "react";

type Tone = "pos" | "warn" | "neg" | "info" | "muted";

type Props = {
  tone?: Tone;
  children: ReactNode;
  className?: string;
};

export default function Badge({ tone = "muted", children, className }: Props) {
  return (
    <span className={`db-badge is-${tone}${className ? ` ${className}` : ""}`}>
      {children}
    </span>
  );
}
