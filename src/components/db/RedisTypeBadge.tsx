type Props = {
  /** Redis key type — raw value from `RedisKeyView.kind`, e.g. "string", "hash", "list", "zset", "stream". */
  kind: string | null | undefined;
};

/** Colored type chip that mirrors the design's `.rds-type.*` palette. */
export default function RedisTypeBadge({ kind }: Props) {
  const normalized = (kind || "").toLowerCase();
  const variant =
    normalized === "string" || normalized === "str"
      ? "str"
      : normalized === "hash"
        ? "hash"
        : normalized === "zset" || normalized === "sortedset"
          ? "zset"
          : normalized === "list"
            ? "list"
            : normalized === "stream"
              ? "stream"
              : "unknown";
  const label =
    variant === "str"
      ? "STR"
      : variant === "hash"
        ? "HASH"
        : variant === "zset"
          ? "ZSET"
          : variant === "list"
            ? "LIST"
            : variant === "stream"
              ? "STREAM"
              : normalized.slice(0, 5).toUpperCase() || "?";
  return <span className={`rds-type rds-type--${variant}`}>{label}</span>;
}
