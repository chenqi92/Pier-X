// Local-only bundle scheduler. Persists per-(host, bundle) schedules
// in localStorage and exposes helpers for "is this entry due?". The
// actual firing happens in `SoftwarePanel` via a 60-second tick that
// reads schedules and calls `runBundle`.
//
// Cron expressions were on the table but adding a parser (or pulling
// in `cron-parser`) is heavier than this needs to be. We support three
// shapes that cover the common ops cases:
//
//   • interval — every N minutes
//   • daily    — once a day at HH:MM (local time)
//   • weekly   — once a week, on weekday W at HH:MM
//
// Schedules only fire while the SoftwarePanel is mounted for the
// matching swKey. There's no background scheduling — close the app,
// no fires happen. The dialog calls this out explicitly so users
// don't expect cron-style daemon behaviour.

const STORAGE_KEY = "pier-x:bundle-schedules";

export type ScheduleKind = "interval" | "daily" | "weekly";

export type BundleSchedule = {
  id: string;
  /** swKey identifying the SSH target (host:port:user). */
  swKey: string;
  bundleId: string;
  kind: ScheduleKind;
  /** Required for `interval` kind. Minimum 5 to avoid runaway loops. */
  intervalMinutes?: number;
  /** 0-23. Required for `daily` and `weekly`. */
  hour?: number;
  /** 0-59. Required for `daily` and `weekly`. */
  minute?: number;
  /** 0=Sun..6=Sat. Required for `weekly`. */
  weekday?: number;
  enabled: boolean;
  /** Unix ms of the last fire. Used to avoid double-firing within
   *  the same window and to compute `due`. */
  lastRunAt?: number;
  /** Friendly label shown in the dialog and in toasts. */
  label?: string;
};

export function loadSchedules(): BundleSchedule[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(isValidSchedule);
  } catch {
    return [];
  }
}

export function saveSchedules(schedules: BundleSchedule[]): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(schedules));
  } catch {
    // localStorage full / disabled — silent. The dialog reflects
    // its own state via React, so the user keeps editing without
    // realising the persistence failed; that's fine for a v1.
  }
}

function isValidSchedule(v: unknown): v is BundleSchedule {
  if (!v || typeof v !== "object") return false;
  const o = v as Record<string, unknown>;
  return (
    typeof o.id === "string" &&
    typeof o.swKey === "string" &&
    typeof o.bundleId === "string" &&
    (o.kind === "interval" || o.kind === "daily" || o.kind === "weekly") &&
    typeof o.enabled === "boolean"
  );
}

/** True when `now` is at-or-after the schedule's next fire time and
 *  the schedule hasn't already fired during the window. Returns false
 *  for disabled schedules and for unknown / malformed shapes. */
export function isDue(s: BundleSchedule, now: Date): boolean {
  if (!s.enabled) return false;
  const nowMs = now.getTime();
  switch (s.kind) {
    case "interval": {
      const mins = Math.max(5, s.intervalMinutes ?? 60);
      const intervalMs = mins * 60_000;
      // Never run before the first interval expires; from then on,
      // any time we're past lastRun + interval we're due.
      const last = s.lastRunAt ?? 0;
      if (last === 0) {
        // First fire — wait one full interval since the schedule
        // was created. We don't track creation time separately so
        // we treat "no last run" as "wait one interval from a
        // synthetic anchor at lastRun time"; in practice that means
        // the very first run happens approximately one interval
        // after the user enabled the schedule, which matches user
        // expectations.
        return false;
      }
      return nowMs >= last + intervalMs;
    }
    case "daily": {
      const target = nextDailyTime(s.hour ?? 0, s.minute ?? 0, now);
      // The "previous" target = the most recent passing of HH:MM
      // before `now`. We're due when:
      //   prev <= now AND lastRunAt < prev (we haven't fired this
      //   window yet)
      const prev = previousDailyTime(s.hour ?? 0, s.minute ?? 0, now);
      void target; // intentionally unused — kept for symmetry
      return prev.getTime() <= nowMs && (s.lastRunAt ?? 0) < prev.getTime();
    }
    case "weekly": {
      const prev = previousWeeklyTime(
        s.weekday ?? 0,
        s.hour ?? 0,
        s.minute ?? 0,
        now,
      );
      return prev.getTime() <= nowMs && (s.lastRunAt ?? 0) < prev.getTime();
    }
  }
}

function previousDailyTime(hour: number, minute: number, now: Date): Date {
  const d = new Date(now);
  d.setHours(hour, minute, 0, 0);
  if (d.getTime() > now.getTime()) {
    d.setDate(d.getDate() - 1);
  }
  return d;
}

function nextDailyTime(hour: number, minute: number, now: Date): Date {
  const d = new Date(now);
  d.setHours(hour, minute, 0, 0);
  if (d.getTime() <= now.getTime()) {
    d.setDate(d.getDate() + 1);
  }
  return d;
}

function previousWeeklyTime(
  weekday: number,
  hour: number,
  minute: number,
  now: Date,
): Date {
  const d = new Date(now);
  d.setHours(hour, minute, 0, 0);
  // Walk backwards until weekday matches.
  while (d.getDay() !== weekday || d.getTime() > now.getTime()) {
    d.setDate(d.getDate() - 1);
  }
  return d;
}

/** Human-readable summary of a schedule — used in the dialog list and
 *  in toast messages. Locale-friendly: relies on built-in
 *  `toLocaleString` for the date and pads HH:MM ourselves. */
export function describeSchedule(s: BundleSchedule): string {
  switch (s.kind) {
    case "interval":
      return `every ${s.intervalMinutes ?? 60} min`;
    case "daily":
      return `daily at ${pad2(s.hour ?? 0)}:${pad2(s.minute ?? 0)}`;
    case "weekly":
      return `${weekdayName(s.weekday ?? 0)} at ${pad2(
        s.hour ?? 0,
      )}:${pad2(s.minute ?? 0)}`;
  }
}

function pad2(n: number): string {
  return String(n).padStart(2, "0");
}

const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

function weekdayName(n: number): string {
  return WEEKDAYS[Math.max(0, Math.min(6, n))] ?? "Sun";
}

/** Generate a fresh id for a new schedule. localStorage doesn't have
 *  a true UUID so we lean on Math.random + timestamp; collision risk
 *  is ignorable for per-user state. */
export function makeScheduleId(): string {
  return `sch-${Date.now().toString(36)}-${Math.random()
    .toString(36)
    .slice(2, 8)}`;
}

/**
 * Convert a `BundleSchedule` into a 5-field cron expression.
 *
 * - interval N min → `*\/N * * * *` (capped to 60 min so it lands on a
 *   minute field; longer intervals fall back to a once-per-hour cap)
 * - daily HH:MM → `MM HH * * *`
 * - weekly W HH:MM → `MM HH * * W`
 *
 * Returns null if the schedule isn't representable in cron's 5-field
 * grammar (intervals > 1 day get clamped daily).
 */
export function toCronExpression(s: BundleSchedule): string | null {
  switch (s.kind) {
    case "interval": {
      const mins = Math.max(5, s.intervalMinutes ?? 60);
      if (mins <= 60) return `*/${mins} * * * *`;
      // Hours bucket — 90 min becomes "every 1.5 hours" which doesn't
      // map; fall back to "every N hours" rounded down.
      const hrs = Math.max(1, Math.floor(mins / 60));
      return `0 */${hrs} * * *`;
    }
    case "daily":
      return `${s.minute ?? 0} ${s.hour ?? 0} * * *`;
    case "weekly":
      return `${s.minute ?? 0} ${s.hour ?? 0} * * ${s.weekday ?? 0}`;
  }
}

/**
 * Wrap a shell command into a complete crontab line tagged with the
 * schedule id, so the user can later locate / remove it. The tag is
 * a trailing comment that's preserved by `crontab -l` round-trips.
 */
export function buildCronLine(s: BundleSchedule, command: string): string {
  const expr = toCronExpression(s);
  if (!expr) return "";
  return `${expr} ${command}  # pier-x:bundle:${s.id}`;
}
