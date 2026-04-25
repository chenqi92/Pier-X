import { useI18n } from "../../i18n/useI18n";
import type { RedisKeyEntry } from "../../lib/types";
import RedisTypeBadge from "./RedisTypeBadge";

type Props = {
  keys: RedisKeyEntry[];
  /** Currently selected key — used to colour the matching row. */
  selected: string | null;
  /** Type of the selected key (preferred over the per-row scan kind, since detail inspection is authoritative). */
  selectedKind?: string | null;
  onSelect: (key: string) => void;
  /** Whether more keys remain on the server — drives the "Load more" button. */
  hasMore?: boolean;
  /** Click handler for "Load more". When omitted, the button is hidden. */
  onLoadMore?: () => void;
  /** Disabled flag for "Load more" while a request is in flight. */
  loadMoreBusy?: boolean;
};

/**
 * Flat key list. Keys come pre-typed from the backend (TYPE pipeline
 * after SCAN), so each row carries its own kind / TTL chip.
 */
export default function RedisKeyList({
  keys,
  selected,
  selectedKind,
  onSelect,
  hasMore,
  onLoadMore,
  loadMoreBusy,
}: Props) {
  const { t } = useI18n();

  if (keys.length === 0) {
    return (
      <div className="rds-detail-empty" style={{ padding: "var(--sp-6) var(--sp-4)" }}>
        <div>{t("No keys match this pattern.")}</div>
      </div>
    );
  }

  return (
    <div className="rds-keys">
      {keys.map((entry) => {
        const isSelected = entry.key === selected;
        const kind = isSelected && selectedKind ? selectedKind : entry.kind;
        return (
          <button
            key={entry.key}
            type="button"
            className={"rds-row" + (isSelected ? " selected" : "")}
            onClick={() => onSelect(entry.key)}
          >
            <RedisTypeBadge kind={kind} />
            <span className="rds-key">{entry.key}</span>
            <span className="rds-meta" title={ttlTitle(entry.ttlSeconds, t)}>
              {formatTtl(entry.ttlSeconds, t)}
            </span>
            <span className="rds-meta" />
          </button>
        );
      })}
      {hasMore && onLoadMore && (
        <button
          type="button"
          className="btn is-ghost is-compact rds-load-more"
          onClick={onLoadMore}
          disabled={loadMoreBusy}
        >
          {loadMoreBusy ? t("Loading...") : t("Load more")}
        </button>
      )}
    </div>
  );
}

type TFn = (key: string, vars?: Record<string, string | number | null | undefined>) => string;

function formatTtl(ttl: number, _t: TFn): string {
  if (ttl === -1) return "∞";
  if (ttl < 0) return "—";
  if (ttl < 60) return `${ttl}s`;
  if (ttl < 3600) return `${Math.round(ttl / 60)}m`;
  if (ttl < 86400) return `${Math.round(ttl / 3600)}h`;
  return `${Math.round(ttl / 86400)}d`;
}

function ttlTitle(ttl: number, t: TFn): string {
  if (ttl === -1) return t("No expiry");
  if (ttl < 0) return t("Unknown TTL");
  return t("Expires in {seconds}s", { seconds: ttl });
}
