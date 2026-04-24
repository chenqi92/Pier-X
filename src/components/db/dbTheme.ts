import { Database, HardDrive, Zap } from "lucide-react";
import type { ComponentType, SVGProps } from "react";

import type { DbKind } from "../../lib/types";

type LucideIcon = ComponentType<SVGProps<SVGSVGElement> & { size?: number | string }>;

/**
 * Per-engine visuals — accent (CSS color expression), process/daemon
 * label used in the splash probe line, and the glyph icon. Intentionally
 * derived from the existing service brand tokens so the panel chrome,
 * toolstrip, and sidebar stay colour-consistent.
 */
export type DbTheme = {
  kind: DbKind;
  /** CSS color expression — `var(--svc-*)`. */
  tintVar: string;
  /** Background tint for the instance-picker icon chip. */
  chipBgVar: string;
  /** Process / daemon label shown in the splash probe line. */
  daemon: string;
  icon: LucideIcon;
};

export const DB_THEMES: Record<DbKind, DbTheme> = {
  mysql: {
    kind: "mysql",
    tintVar: "var(--svc-mysql)",
    chipBgVar: "color-mix(in srgb, var(--svc-mysql) 18%, transparent)",
    daemon: "mysqld",
    icon: Database,
  },
  postgres: {
    kind: "postgres",
    tintVar: "var(--svc-postgres)",
    chipBgVar: "color-mix(in srgb, var(--svc-postgres) 18%, transparent)",
    daemon: "postgres",
    icon: Database,
  },
  redis: {
    kind: "redis",
    tintVar: "var(--svc-redis)",
    chipBgVar: "color-mix(in srgb, var(--svc-redis) 18%, transparent)",
    daemon: "redis-server",
    icon: Zap,
  },
  sqlite: {
    kind: "sqlite",
    tintVar: "var(--svc-sqlite)",
    chipBgVar: "color-mix(in srgb, var(--svc-sqlite) 18%, transparent)",
    daemon: "libsqlite3",
    icon: HardDrive,
  },
};

/**
 * Infer the environment tag (`prod` / `stage` / `dev` / `local`) from a
 * saved-credential label. Purely a visual hint — the backend does not
 * yet persist an explicit tag (see docs/BACKEND-GAPS.md).
 */
export type DbEnv = "prod" | "stage" | "dev" | "local" | "unknown";

export function inferEnv(label: string | undefined | null): DbEnv {
  const hay = (label ?? "").toLowerCase();
  if (/\bprod(uction)?\b/.test(hay)) return "prod";
  if (/\b(stage|staging)\b/.test(hay)) return "stage";
  if (/\b(dev|qa|test|sandbox)\b/.test(hay)) return "dev";
  if (/\b(local|localhost)\b/.test(hay)) return "local";
  return "unknown";
}
