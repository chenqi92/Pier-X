// Small visual chip for SSH connection environment tags. The
// `prod` / `staging` / `dev` / `local` values get distinctive
// colors so a prod box stands out from staging at a glance; any
// other free-form tag falls back to a neutral pill.

type Props = {
  tag: string | null | undefined;
  /** When true, renders a tighter pill suitable for dense table
   *  cells. Default is the regular pill used in cards / headers. */
  compact?: boolean;
};

const KNOWN_TONES: Record<string, string> = {
  prod: "env-tag--prod",
  production: "env-tag--prod",
  staging: "env-tag--staging",
  stage: "env-tag--staging",
  uat: "env-tag--staging",
  dev: "env-tag--dev",
  development: "env-tag--dev",
  test: "env-tag--dev",
  local: "env-tag--local",
};

export default function EnvTagChip({ tag, compact }: Props) {
  if (!tag) return null;
  const trimmed = tag.trim();
  if (!trimmed) return null;
  const tone = KNOWN_TONES[trimmed.toLowerCase()] ?? "env-tag--neutral";
  return (
    <span
      className={`env-tag mono ${tone} ${compact ? "env-tag--compact" : ""}`}
      title={trimmed}
    >
      {trimmed}
    </span>
  );
}
