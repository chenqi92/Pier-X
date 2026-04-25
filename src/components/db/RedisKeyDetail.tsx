import { Edit, Trash2, Zap } from "lucide-react";

import { useI18n } from "../../i18n/useI18n";
import RedisTypeBadge from "./RedisTypeBadge";
import type { RedisKeyView } from "../../lib/types";

type Props = {
  details: RedisKeyView | null;
  /** When provided, the head shows a Rename action that opens a
   *  prompt with the current key name pre-filled. The handler
   *  receives the new name and is responsible for the round-
   *  trip + reload. */
  onRename?: (currentKey: string, nextKey: string) => void;
  /** When provided, the head shows a Delete action guarded by a
   *  confirm() dialog. */
  onDelete?: (key: string) => void;
  /** Disabled flag while a Rename / Delete is in flight. */
  actionBusy?: boolean;
};

/**
 * Right-pane viewer for the selected Redis key. Read-only in the
 * pilot — inline edit / SET / HSET / LPUSH flows are design-only
 * placeholders (see docs/BACKEND-GAPS.md).
 */
export default function RedisKeyDetail({ details, onRename, onDelete, actionBusy }: Props) {
  const { t } = useI18n();

  if (!details) {
    return (
      <div className="rds-detail-empty">
        <Zap size={22} />
        <div>{t("Select a key to view its value.")}</div>
      </div>
    );
  }

  const ttlLabel =
    details.ttlSeconds < 0
      ? t("persistent")
      : t("{seconds}s", { seconds: details.ttlSeconds });

  const kind = (details.kind || "").toLowerCase();
  const isHash = kind === "hash";
  const isList = kind === "list";
  const isZset = kind === "zset";
  const isStream = kind === "stream";

  return (
    <>
      <div className="rds-detail-head">
        <RedisTypeBadge kind={details.kind} />
        <span className="rds-detail-key">{details.key}</span>
        <span style={{ flex: 1 }} />
        {onRename && (
          <button
            type="button"
            className="btn is-ghost is-compact"
            disabled={actionBusy}
            onClick={() => {
              const next = window.prompt(t("Rename key — enter a new name:"), details.key);
              if (next == null) return;
              const trimmed = next.trim();
              if (!trimmed || trimmed === details.key) return;
              onRename(details.key, trimmed);
            }}
            title={t("Rename")}
          >
            <Edit size={10} /> {t("Rename")}
          </button>
        )}
        {onDelete && (
          <button
            type="button"
            className="btn is-ghost is-compact is-danger"
            disabled={actionBusy}
            onClick={() => {
              const ok = window.confirm(
                t("Delete key {key}? This cannot be undone.", { key: details.key }),
              );
              if (!ok) return;
              onDelete(details.key);
            }}
            title={t("Delete")}
          >
            <Trash2 size={10} /> {t("Delete")}
          </button>
        )}
      </div>
      <div className="rds-detail-meta">
        <span>
          {t("TTL")} <b>{ttlLabel}</b>
        </span>
        <span className="sep">·</span>
        <span>
          {t("LENGTH")} <b>{details.length.toLocaleString()}</b>
        </span>
        {details.encoding && (
          <>
            <span className="sep">·</span>
            <span>
              {t("ENC")} <b>{details.encoding}</b>
            </span>
          </>
        )}
      </div>

      {details.preview.length === 0 ? (
        <div className="rds-detail-empty" style={{ padding: "var(--sp-6) var(--sp-4)" }}>
          <div>{t("(empty)")}</div>
        </div>
      ) : isHash ? (
        <HashView preview={details.preview} />
      ) : isList || isZset || isStream ? (
        <ListView preview={details.preview} />
      ) : (
        <StringView preview={details.preview} />
      )}

      {details.previewTruncated && (
        <div className="rds-truncated-note">
          {t("Preview truncated — the value continues beyond what's shown.")}
        </div>
      )}
    </>
  );
}

function HashView({ preview }: { preview: string[] }) {
  const { t } = useI18n();
  const pairs: [string, string][] = [];
  for (let i = 0; i < preview.length; i += 2) {
    pairs.push([preview[i], preview[i + 1] ?? ""]);
  }
  return (
    <div className="rds-kv">
      <div className="rds-kv-head">
        <span>{t("FIELD")}</span>
        <span>{t("VALUE")}</span>
      </div>
      {pairs.map(([field, value], i) => (
        <div key={`${field}-${i}`} className="rds-kv-row">
          <span className="rds-kv-field">{field}</span>
          <span className="rds-kv-value">{value}</span>
        </div>
      ))}
    </div>
  );
}

function ListView({ preview }: { preview: string[] }) {
  const { t } = useI18n();
  return (
    <div className="rds-kv">
      <div className="rds-kv-head">
        <span>#</span>
        <span>{t("ELEMENT")}</span>
      </div>
      {preview.map((value, i) => (
        <div key={i} className="rds-kv-row">
          <span className="rds-kv-field">{i}</span>
          <span className="rds-kv-value">{value}</span>
        </div>
      ))}
    </div>
  );
}

function StringView({ preview }: { preview: string[] }) {
  const { t } = useI18n();
  return (
    <div className="rds-value">
      <div className="rds-value-head">{t("VALUE")}</div>
      <div className="rds-value-body">{preview.join("\n")}</div>
    </div>
  );
}
