type Tone = "pos" | "off" | "warn" | "neg";

type Props = {
  tone?: Tone;
  className?: string;
};

export default function StatusDot({ tone = "pos", className }: Props) {
  return <span className={`gb-dot is-${tone}${className ? ` ${className}` : ""}`} />;
}
