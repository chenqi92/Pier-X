import type { ReactNode } from "react";

type Props = {
  children: ReactNode;
  className?: string;
};

export default function KeyCap({ children, className }: Props) {
  return <kbd className={`kbd${className ? ` ${className}` : ""}`}>{children}</kbd>;
}
