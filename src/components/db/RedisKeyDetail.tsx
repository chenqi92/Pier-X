import { Check, Edit, Plus, Save, Search, Trash2, X } from "lucide-react";
import RedisIcon from "../icons/RedisIcon";
import { useEffect, useMemo, useRef, useState } from "react";

import { useI18n } from "../../i18n/useI18n";
import Select, { type SelectItems } from "../Select";
import RedisTypeBadge from "./RedisTypeBadge";
import type { RedisKeyView } from "../../lib/types";
import { confirm } from "../../stores/useConfirmStore";

/** Discriminated edit operation the panel knows how to translate
 *  into a redis command. The detail view never calls Redis directly
 *  — it just emits these and lets the panel build the SQL+ssh probe
 *  + refetch + error surfacing. */
export type RedisEdit =
  | { kind: "string-set"; value: string }
  | { kind: "hash-set"; field: string; value: string }
  | { kind: "hash-del"; field: string }
  | { kind: "list-set"; index: number; value: string }
  | { kind: "list-push"; side: "L" | "R"; value: string }
  /** Index-precise removal — the panel runs an atomic tombstone swap
   *  so duplicate values don't make `LREM` delete the wrong row. */
  | { kind: "list-rem"; index: number }
  | { kind: "set-add"; member: string }
  | { kind: "set-rem"; member: string }
  | { kind: "zset-add"; score: number; member: string }
  | { kind: "zset-rem"; member: string }
  /** `seconds === null` ⇒ PERSIST. `0` would EXPIRE-now (delete);
   *  the panel rejects 0 / negatives at the form level. */
  | { kind: "ttl-set"; seconds: number | null };

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
  /** When provided, the value pane gets per-type edit affordances
   *  (string textarea, hash field add/edit/delete, list push/edit/
   *  remove, set / zset add+remove, TTL EXPIRE/PERSIST). The
   *  callback runs the underlying redis command + reloads. */
  onEdit?: (op: RedisEdit) => Promise<void>;
  /** Disabled flag while a Rename / Delete / Edit is in flight. */
  actionBusy?: boolean;
  /** Loads the next page of a collection key's entries (hash / set /
   *  list / zset / stream) and appends them to `details.preview`.
   *  When provided, collection views show a "Load more" button while
   *  `details.previewTruncated` holds. */
  onLoadMoreEntries?: () => void;
  /** Disabled flag while an entry-page load is in flight. */
  entriesBusy?: boolean;
};

/**
 * Right-pane viewer for the selected Redis key. When `onEdit` is
 * provided, every visible piece of the value becomes mutable —
 * string SET, hash HSET/HDEL, list LSET/LPUSH/RPUSH/LREM, set
 * SADD/SREM, zset ZADD/ZREM, plus EXPIRE/PERSIST for TTL. Stream
 * stays read-only (XADD's field=value pairs need a more elaborate
 * form than this view).
 */
export default function RedisKeyDetail({
  details,
  onRename,
  onDelete,
  onEdit,
  actionBusy,
  onLoadMoreEntries,
  entriesBusy,
}: Props) {
  const { t } = useI18n();

  if (!details) {
    return (
      <div className="rds-detail-empty">
        <RedisIcon size={22} />
        <div>{t("Select a key to view its value.")}</div>
      </div>
    );
  }

  const kind = (details.kind || "").toLowerCase();
  const isHash = kind === "hash";
  const isList = kind === "list";
  const isSet = kind === "set";
  const isZset = kind === "zset";
  const isStream = kind === "stream";
  const isString = !isHash && !isList && !isSet && !isZset && !isStream;
  const editEnabled = !!onEdit && !actionBusy;

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
            onClick={async () => {
              const ok = await confirm({
                message: t("Delete key {key}? This cannot be undone.", { key: details.key }),
                tone: "destructive",
              });
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
        <TtlChip
          ttlSeconds={details.ttlSeconds}
          editable={editEnabled}
          onCommit={(seconds) => onEdit?.({ kind: "ttl-set", seconds })}
        />
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

      {/* Key the view on the key name so switching keys remounts it,
          clearing per-key view state (filter text, value lens, inline
          edit drafts) instead of leaking it onto the next key. */}
      {isString ? (
        <StringView
          key={details.key}
          preview={details.preview}
          onEdit={editEnabled ? onEdit : undefined}
        />
      ) : isHash ? (
        <HashView
          key={details.key}
          preview={details.preview}
          total={details.length}
          onEdit={editEnabled ? onEdit : undefined}
        />
      ) : isList ? (
        <ListView
          key={details.key}
          preview={details.preview}
          total={details.length}
          onEdit={editEnabled ? onEdit : undefined}
        />
      ) : isSet ? (
        <SetView
          key={details.key}
          preview={details.preview}
          total={details.length}
          onEdit={editEnabled ? onEdit : undefined}
        />
      ) : isZset ? (
        <ZsetView
          key={details.key}
          preview={details.preview}
          total={details.length}
          onEdit={editEnabled ? onEdit : undefined}
        />
      ) : (
        // Stream — keep read-only for now; XADD's field/value
        // pairs need a more elaborate form than this view.
        <StreamView key={details.key} preview={details.preview} total={details.length} />
      )}

      {!isString && onLoadMoreEntries && details.previewTruncated ? (
        <LoadMoreEntries onClick={onLoadMoreEntries} busy={entriesBusy} />
      ) : isString && details.previewTruncated ? (
        <div className="rds-truncated-note">
          {t("Preview truncated — the value continues beyond what's shown.")}
        </div>
      ) : null}
    </>
  );
}

// ── TTL ──────────────────────────────────────────────────────────

function TtlChip({
  ttlSeconds,
  editable,
  onCommit,
}: {
  ttlSeconds: number;
  editable: boolean;
  onCommit?: (seconds: number | null) => Promise<void> | void;
}) {
  const { t } = useI18n();
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const inputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    if (editing) inputRef.current?.focus();
  }, [editing]);

  const ttlLabel =
    ttlSeconds < 0 ? t("persistent") : t("{seconds}s", { seconds: ttlSeconds });

  if (!editing) {
    return (
      <span className="rds-ttl-chip">
        {t("TTL")} <b>{ttlLabel}</b>
        {editable && onCommit && (
          <button
            type="button"
            className="mini-button mini-button--ghost rds-ttl-edit"
            onClick={() => {
              setDraft(ttlSeconds < 0 ? "" : String(ttlSeconds));
              setEditing(true);
            }}
            title={t("Set EXPIRE / PERSIST")}
          >
            <Edit size={9} />
          </button>
        )}
      </span>
    );
  }

  const parsed = Number.parseInt(draft.trim(), 10);
  const validExpire = Number.isFinite(parsed) && parsed > 0;

  return (
    <span className="rds-ttl-chip">
      {t("TTL")}
      <input
        ref={inputRef}
        className="rds-input rds-input--narrow"
        value={draft}
        placeholder={t("seconds")}
        onChange={(e) => setDraft(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            setEditing(false);
          } else if (e.key === "Enter" && validExpire) {
            void onCommit?.(parsed);
            setEditing(false);
          }
        }}
      />
      <button
        type="button"
        className="mini-button mini-button--ghost"
        disabled={!validExpire}
        onClick={() => {
          void onCommit?.(parsed);
          setEditing(false);
        }}
        title={t("EXPIRE")}
      >
        <Check size={9} />
      </button>
      <button
        type="button"
        className="mini-button mini-button--ghost"
        onClick={() => {
          void onCommit?.(null);
          setEditing(false);
        }}
        title={t("PERSIST (clear TTL)")}
      >
        <Save size={9} />
      </button>
      <button
        type="button"
        className="mini-button mini-button--ghost"
        onClick={() => setEditing(false)}
        title={t("Cancel")}
      >
        <X size={9} />
      </button>
    </span>
  );
}

// ── String ───────────────────────────────────────────────────────

/** Display lens for a string value. The raw bytes are never mutated —
 *  these only re-render how the read-only preview is shown, matching
 *  the "view as" affordance other Redis GUIs expose. */
type ValueFormat = "text" | "json" | "hex" | "base64";

/** Auto-pick JSON when the value is obviously a JSON document so the
 *  common case (cached API payloads) lands pretty-printed; otherwise
 *  fall back to the raw text lens. */
function detectFormat(raw: string): ValueFormat {
  const s = raw.trim();
  const looksJson =
    (s.startsWith("{") && s.endsWith("}")) || (s.startsWith("[") && s.endsWith("]"));
  if (looksJson) {
    try {
      JSON.parse(s);
      return "json";
    } catch {
      /* not actually JSON — fall through */
    }
  }
  return "text";
}

function toHexDump(raw: string): string {
  const bytes = new TextEncoder().encode(raw);
  const lines: string[] = [];
  for (let i = 0; i < bytes.length; i += 16) {
    const slice = bytes.subarray(i, i + 16);
    const hex = Array.from(slice, (b) => b.toString(16).padStart(2, "0")).join(" ");
    const ascii = Array.from(slice, (b) =>
      b >= 0x20 && b < 0x7f ? String.fromCharCode(b) : ".",
    ).join("");
    const off = i.toString(16).padStart(8, "0");
    lines.push(`${off}  ${hex.padEnd(47, " ")}  ${ascii}`);
  }
  return lines.join("\n");
}

function decodeBase64(raw: string): string {
  const bin = atob(raw.trim());
  const bytes = Uint8Array.from(bin, (c) => c.charCodeAt(0));
  return new TextDecoder("utf-8", { fatal: false }).decode(bytes);
}

function applyFormat(
  raw: string,
  fmt: ValueFormat,
  t: (key: string) => string,
): { text: string; error?: string } {
  switch (fmt) {
    case "json":
      try {
        return { text: JSON.stringify(JSON.parse(raw), null, 2) };
      } catch {
        return { text: raw, error: t("Not valid JSON") };
      }
    case "hex":
      return { text: toHexDump(raw) };
    case "base64":
      try {
        return { text: decodeBase64(raw) };
      } catch {
        return { text: raw, error: t("Not valid Base64") };
      }
    case "text":
    default:
      return { text: raw };
  }
}

// ── Collection chrome (shared by hash / list / set / zset / stream) ─

const FORMAT_ITEMS = (t: (key: string) => string): SelectItems => [
  { value: "text", label: t("Plain Text") },
  { value: "json", label: t("JSON") },
  { value: "hex", label: t("Hex") },
  { value: "base64", label: t("Base64") },
];

/** Filter + value-lens state shared by every collection view. The
 *  filter narrows the *loaded* entries (substring, case-insensitive);
 *  the lens reformats the value column (Plain Text / JSON / Hex /
 *  Base64) the same way the string view does. */
function useEntryView() {
  const { t } = useI18n();
  const [query, setQuery] = useState("");
  const [format, setFormat] = useState<ValueFormat>("text");
  const fmt = (value: string) => applyFormat(value, format, t).text;
  const match = (...fields: string[]) => {
    const q = query.trim().toLowerCase();
    if (!q) return true;
    return fields.some((f) => f.toLowerCase().includes(q));
  };
  return { query, setQuery, format, setFormat, fmt, match };
}

function CollectionToolbar({
  query,
  onQuery,
  format,
  onFormat,
  loaded,
  total,
}: {
  query: string;
  onQuery: (v: string) => void;
  format: ValueFormat;
  onFormat: (f: ValueFormat) => void;
  loaded: number;
  total: number;
}) {
  const { t } = useI18n();
  return (
    <div className="rds-kv-toolbar">
      <span className="rds-kv-search">
        <Search size={11} aria-hidden />
        <input
          className="rds-kv-search-input"
          placeholder={t("Filter loaded entries…")}
          value={query}
          onChange={(e) => onQuery(e.currentTarget.value)}
        />
        {query && (
          <button
            type="button"
            className="mini-button mini-button--ghost"
            onClick={() => onQuery("")}
            title={t("Clear filter")}
          >
            <X size={9} />
          </button>
        )}
      </span>
      <span className="rds-kv-count" title={t("Loaded / total")}>
        {total > loaded ? `${loaded.toLocaleString()} / ${total.toLocaleString()}` : loaded.toLocaleString()}
      </span>
      <Select
        compact
        mono
        items={FORMAT_ITEMS(t)}
        value={format}
        onChange={(v) => onFormat(v as ValueFormat)}
        title={t("View as")}
      />
    </div>
  );
}

function LoadMoreEntries({ onClick, busy }: { onClick: () => void; busy?: boolean }) {
  const { t } = useI18n();
  return (
    <button
      type="button"
      className="btn is-ghost is-compact rds-load-more"
      onClick={onClick}
      disabled={busy}
    >
      {busy ? t("Loading...") : t("Load more entries")}
    </button>
  );
}

function StringView({
  preview,
  onEdit,
}: {
  preview: string[];
  onEdit?: (op: RedisEdit) => Promise<void>;
}) {
  const { t } = useI18n();
  const original = preview.join("\n");
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(original);
  const [format, setFormat] = useState<ValueFormat>(() => detectFormat(original));
  // Reset the draft / lens whenever the source value changes —
  // typically after a successful Save the panel re-fetches the key
  // and we get a fresh `preview` here.
  useEffect(() => {
    setDraft(original);
    setEditing(false);
    setFormat(detectFormat(original));
  }, [original]);

  const formatItems = useMemo(() => FORMAT_ITEMS(t), [t]);
  const shown = useMemo(() => applyFormat(original, format, t), [original, format, t]);

  if (!editing) {
    return (
      <div className="rds-value">
        <div className="rds-value-head">
          {t("VALUE")}
          <span className="rds-value-tools">
            {shown.error && <span className="rds-value-format-err">{shown.error}</span>}
            <Select
              compact
              mono
              items={formatItems}
              value={format}
              onChange={(v) => setFormat(v as ValueFormat)}
              title={t("View as")}
            />
            {onEdit && (
              <button
                type="button"
                className="mini-button mini-button--ghost"
                onClick={() => setEditing(true)}
                title={t("Edit value")}
              >
                <Edit size={9} /> {t("Edit")}
              </button>
            )}
          </span>
        </div>
        <div className="rds-value-body">{shown.text}</div>
      </div>
    );
  }

  return (
    <div className="rds-value">
      <div className="rds-value-head">{t("VALUE")}</div>
      <textarea
        className="rds-value-edit"
        value={draft}
        autoFocus
        onChange={(e) => setDraft(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            setEditing(false);
            setDraft(original);
          }
        }}
      />
      <div className="rds-edit-foot">
        <button
          type="button"
          className="btn is-ghost is-compact"
          onClick={() => {
            setEditing(false);
            setDraft(original);
          }}
        >
          <X size={10} /> {t("Cancel")}
        </button>
        <button
          type="button"
          className="btn is-primary is-compact"
          disabled={draft === original}
          onClick={() => void onEdit?.({ kind: "string-set", value: draft })}
        >
          <Save size={10} /> {t("SET")}
        </button>
      </div>
    </div>
  );
}

// ── Hash ─────────────────────────────────────────────────────────

function HashView({
  preview,
  total,
  onEdit,
}: {
  preview: string[];
  total: number;
  onEdit?: (op: RedisEdit) => Promise<void>;
}) {
  const { t } = useI18n();
  const { query, setQuery, format, setFormat, fmt, match } = useEntryView();
  // The backend serializes each hash entry as `field \x01 value`
  // (one string per field, SOH separator). Split on the first \x01;
  // field names never contain it, so field/value are unambiguous.
  const pairs: [string, string][] = preview.map((line) => {
    const idx = line.indexOf("\u0001");
    return idx === -1 ? [line, ""] : [line.slice(0, idx), line.slice(idx + 1)];
  });
  const visible = pairs.filter(([field, value]) => match(field, value));
  const [editingField, setEditingField] = useState<string | null>(null);
  const [editDraft, setEditDraft] = useState("");
  const [newField, setNewField] = useState("");
  const [newValue, setNewValue] = useState("");

  return (
    <div className="rds-kv">
      <CollectionToolbar
        query={query}
        onQuery={setQuery}
        format={format}
        onFormat={setFormat}
        loaded={pairs.length}
        total={total}
      />
      <div className="rds-kv-head">
        <span>{t("FIELD")}</span>
        <span>{t("VALUE")}</span>
        {onEdit && <span className="rds-kv-actions" aria-hidden />}
      </div>
      {visible.map(([field, value], i) => {
        const isEditing = editingField === field;
        return (
          <div key={`${field}-${i}`} className="rds-kv-row">
            <span className="rds-kv-field" title={field}>{field}</span>
            {isEditing ? (
              <input
                className="rds-input"
                value={editDraft}
                autoFocus
                onChange={(e) => setEditDraft(e.currentTarget.value)}
                onKeyDown={(e) => {
                  if (e.key === "Escape") setEditingField(null);
                  else if (e.key === "Enter" && editDraft !== value) {
                    void onEdit?.({ kind: "hash-set", field, value: editDraft });
                    setEditingField(null);
                  }
                }}
              />
            ) : (
              <span className="rds-kv-value">{fmt(value)}</span>
            )}
            {onEdit && (
              <span className="rds-kv-actions">
                {isEditing ? (
                  <>
                    <button
                      type="button"
                      className="mini-button mini-button--ghost"
                      disabled={editDraft === value}
                      onClick={() => {
                        void onEdit({ kind: "hash-set", field, value: editDraft });
                        setEditingField(null);
                      }}
                    >
                      <Check size={9} />
                    </button>
                    <button
                      type="button"
                      className="mini-button mini-button--ghost"
                      onClick={() => setEditingField(null)}
                    >
                      <X size={9} />
                    </button>
                  </>
                ) : (
                  <>
                    <button
                      type="button"
                      className="mini-button mini-button--ghost"
                      onClick={() => {
                        setEditingField(field);
                        setEditDraft(value);
                      }}
                      title={t("Edit value (HSET)")}
                    >
                      <Edit size={9} />
                    </button>
                    <button
                      type="button"
                      className="mini-button mini-button--ghost"
                      onClick={async () => {
                        if (
                          await confirm({ message: t("Delete field {field}?", { field }), tone: "destructive" })
                        ) {
                          void onEdit({ kind: "hash-del", field });
                        }
                      }}
                      title={t("Delete field (HDEL)")}
                    >
                      <Trash2 size={9} />
                    </button>
                  </>
                )}
              </span>
            )}
          </div>
        );
      })}
      {onEdit && (
        <div className="rds-kv-add">
          <input
            className="rds-input"
            placeholder={t("field")}
            value={newField}
            onChange={(e) => setNewField(e.currentTarget.value)}
          />
          <input
            className="rds-input"
            placeholder={t("value")}
            value={newValue}
            onChange={(e) => setNewValue(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && newField.trim()) {
                void onEdit({ kind: "hash-set", field: newField.trim(), value: newValue });
                setNewField("");
                setNewValue("");
              }
            }}
          />
          <button
            type="button"
            className="btn is-ghost is-compact"
            disabled={newField.trim() === ""}
            onClick={() => {
              void onEdit({ kind: "hash-set", field: newField.trim(), value: newValue });
              setNewField("");
              setNewValue("");
            }}
          >
            <Plus size={10} /> {t("HSET")}
          </button>
        </div>
      )}
    </div>
  );
}

// ── List ─────────────────────────────────────────────────────────

function ListView({
  preview,
  total,
  onEdit,
}: {
  preview: string[];
  total: number;
  onEdit?: (op: RedisEdit) => Promise<void>;
}) {
  const { t } = useI18n();
  const { query, setQuery, format, setFormat, fmt, match } = useEntryView();
  const [editingIdx, setEditingIdx] = useState<number | null>(null);
  const [editDraft, setEditDraft] = useState("");
  const [newValue, setNewValue] = useState("");
  // Keep the *original* index — LSET / LREM address elements by it,
  // so the filter must not renumber the rows.
  const rows = preview.map((value, i) => ({ value, i })).filter(({ value }) => match(value));

  return (
    <div className="rds-kv">
      <CollectionToolbar
        query={query}
        onQuery={setQuery}
        format={format}
        onFormat={setFormat}
        loaded={preview.length}
        total={total}
      />
      <div className="rds-kv-head">
        <span>#</span>
        <span>{t("ELEMENT")}</span>
        {onEdit && <span className="rds-kv-actions" aria-hidden />}
      </div>
      {rows.map(({ value, i }) => {
        const isEditing = editingIdx === i;
        return (
          <div key={i} className="rds-kv-row">
            <span className="rds-kv-field">{i}</span>
            {isEditing ? (
              <input
                className="rds-input"
                value={editDraft}
                autoFocus
                onChange={(e) => setEditDraft(e.currentTarget.value)}
                onKeyDown={(e) => {
                  if (e.key === "Escape") setEditingIdx(null);
                  else if (e.key === "Enter" && editDraft !== value) {
                    void onEdit?.({ kind: "list-set", index: i, value: editDraft });
                    setEditingIdx(null);
                  }
                }}
              />
            ) : (
              <span className="rds-kv-value">{fmt(value)}</span>
            )}
            {onEdit && (
              <span className="rds-kv-actions">
                {isEditing ? (
                  <>
                    <button
                      type="button"
                      className="mini-button mini-button--ghost"
                      disabled={editDraft === value}
                      onClick={() => {
                        void onEdit({ kind: "list-set", index: i, value: editDraft });
                        setEditingIdx(null);
                      }}
                    >
                      <Check size={9} />
                    </button>
                    <button
                      type="button"
                      className="mini-button mini-button--ghost"
                      onClick={() => setEditingIdx(null)}
                    >
                      <X size={9} />
                    </button>
                  </>
                ) : (
                  <>
                    <button
                      type="button"
                      className="mini-button mini-button--ghost"
                      onClick={() => {
                        setEditingIdx(i);
                        setEditDraft(value);
                      }}
                      title={t("Edit element (LSET)")}
                    >
                      <Edit size={9} />
                    </button>
                    <button
                      type="button"
                      className="mini-button mini-button--ghost"
                      onClick={async () => {
                        if (await confirm({ message: t("Remove element at index {index}?", { index: i }), tone: "destructive" })) {
                          void onEdit({ kind: "list-rem", index: i });
                        }
                      }}
                      title={t("Remove element at this index")}
                    >
                      <Trash2 size={9} />
                    </button>
                  </>
                )}
              </span>
            )}
          </div>
        );
      })}
      {onEdit && (
        <div className="rds-kv-add">
          <input
            className="rds-input"
            placeholder={t("value")}
            value={newValue}
            onChange={(e) => setNewValue(e.currentTarget.value)}
          />
          <button
            type="button"
            className="btn is-ghost is-compact"
            disabled={newValue === ""}
            onClick={() => {
              void onEdit({ kind: "list-push", side: "L", value: newValue });
              setNewValue("");
            }}
            title={t("Prepend to head (LPUSH)")}
          >
            ← {t("LPUSH")}
          </button>
          <button
            type="button"
            className="btn is-ghost is-compact"
            disabled={newValue === ""}
            onClick={() => {
              void onEdit({ kind: "list-push", side: "R", value: newValue });
              setNewValue("");
            }}
            title={t("Append to tail (RPUSH)")}
          >
            {t("RPUSH")} →
          </button>
        </div>
      )}
    </div>
  );
}

// ── Set ──────────────────────────────────────────────────────────

function SetView({
  preview,
  total,
  onEdit,
}: {
  preview: string[];
  total: number;
  onEdit?: (op: RedisEdit) => Promise<void>;
}) {
  const { t } = useI18n();
  const { query, setQuery, format, setFormat, fmt, match } = useEntryView();
  const [newMember, setNewMember] = useState("");
  const rows = preview.map((member, i) => ({ member, i })).filter(({ member }) => match(member));

  return (
    <div className="rds-kv">
      <CollectionToolbar
        query={query}
        onQuery={setQuery}
        format={format}
        onFormat={setFormat}
        loaded={preview.length}
        total={total}
      />
      <div className="rds-kv-head">
        <span>#</span>
        <span>{t("MEMBER")}</span>
        {onEdit && <span className="rds-kv-actions" aria-hidden />}
      </div>
      {rows.map(({ member, i }) => (
        <div key={`${member}-${i}`} className="rds-kv-row">
          <span className="rds-kv-field">{i}</span>
          <span className="rds-kv-value">{fmt(member)}</span>
          {onEdit && (
            <span className="rds-kv-actions">
              <button
                type="button"
                className="mini-button mini-button--ghost"
                onClick={async () => {
                  if (await confirm({ message: t("Remove member {member}?", { member }), tone: "destructive" })) {
                    void onEdit({ kind: "set-rem", member });
                  }
                }}
                title={t("Remove (SREM)")}
              >
                <Trash2 size={9} />
              </button>
            </span>
          )}
        </div>
      ))}
      {onEdit && (
        <div className="rds-kv-add">
          <input
            className="rds-input"
            placeholder={t("member")}
            value={newMember}
            onChange={(e) => setNewMember(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && newMember.trim()) {
                void onEdit({ kind: "set-add", member: newMember.trim() });
                setNewMember("");
              }
            }}
          />
          <button
            type="button"
            className="btn is-ghost is-compact"
            disabled={newMember.trim() === ""}
            onClick={() => {
              void onEdit({ kind: "set-add", member: newMember.trim() });
              setNewMember("");
            }}
          >
            <Plus size={10} /> {t("SADD")}
          </button>
        </div>
      )}
    </div>
  );
}

// ── Sorted Set ───────────────────────────────────────────────────

/** Backend formats zset preview as `"{score}  {member}"` (two
 *  spaces). Split on the FIRST double-space — score is a default-
 *  formatted f64 so it never has internal spaces. */
function parseZsetEntry(s: string): { score: string; member: string } {
  const idx = s.indexOf("  ");
  if (idx === -1) return { score: "", member: s };
  return { score: s.slice(0, idx), member: s.slice(idx + 2) };
}

function ZsetView({
  preview,
  total,
  onEdit,
}: {
  preview: string[];
  total: number;
  onEdit?: (op: RedisEdit) => Promise<void>;
}) {
  const { t } = useI18n();
  const { query, setQuery, format, setFormat, fmt, match } = useEntryView();
  const entries = preview.map(parseZsetEntry).filter(({ score, member }) => match(member, score));
  const [editingMember, setEditingMember] = useState<string | null>(null);
  const [editDraft, setEditDraft] = useState("");
  const [newMember, setNewMember] = useState("");
  const [newScore, setNewScore] = useState("");

  return (
    <div className="rds-kv">
      <CollectionToolbar
        query={query}
        onQuery={setQuery}
        format={format}
        onFormat={setFormat}
        loaded={preview.length}
        total={total}
      />
      <div className="rds-kv-head">
        <span>{t("SCORE")}</span>
        <span>{t("MEMBER")}</span>
        {onEdit && <span className="rds-kv-actions" aria-hidden />}
      </div>
      {entries.map(({ score, member }, i) => {
        const isEditing = editingMember === member;
        return (
          <div key={`${member}-${i}`} className="rds-kv-row">
            {isEditing ? (
              <input
                className="rds-input"
                value={editDraft}
                autoFocus
                inputMode="decimal"
                onChange={(e) => setEditDraft(e.currentTarget.value)}
                onKeyDown={(e) => {
                  const next = Number.parseFloat(editDraft);
                  if (e.key === "Escape") setEditingMember(null);
                  else if (e.key === "Enter" && Number.isFinite(next)) {
                    void onEdit?.({ kind: "zset-add", score: next, member });
                    setEditingMember(null);
                  }
                }}
              />
            ) : (
              <span className="rds-kv-field">{score}</span>
            )}
            <span className="rds-kv-value">{fmt(member)}</span>
            {onEdit && (
              <span className="rds-kv-actions">
                {isEditing ? (
                  <>
                    <button
                      type="button"
                      className="mini-button mini-button--ghost"
                      disabled={!Number.isFinite(Number.parseFloat(editDraft))}
                      onClick={() => {
                        const next = Number.parseFloat(editDraft);
                        if (Number.isFinite(next)) {
                          void onEdit({ kind: "zset-add", score: next, member });
                          setEditingMember(null);
                        }
                      }}
                    >
                      <Check size={9} />
                    </button>
                    <button
                      type="button"
                      className="mini-button mini-button--ghost"
                      onClick={() => setEditingMember(null)}
                    >
                      <X size={9} />
                    </button>
                  </>
                ) : (
                  <>
                    <button
                      type="button"
                      className="mini-button mini-button--ghost"
                      onClick={() => {
                        setEditingMember(member);
                        setEditDraft(score);
                      }}
                      title={t("Edit score (ZADD)")}
                    >
                      <Edit size={9} />
                    </button>
                    <button
                      type="button"
                      className="mini-button mini-button--ghost"
                      onClick={async () => {
                        if (await confirm({ message: t("Remove member {member}?", { member }), tone: "destructive" })) {
                          void onEdit({ kind: "zset-rem", member });
                        }
                      }}
                      title={t("Remove (ZREM)")}
                    >
                      <Trash2 size={9} />
                    </button>
                  </>
                )}
              </span>
            )}
          </div>
        );
      })}
      {onEdit && (
        <div className="rds-kv-add">
          <input
            className="rds-input rds-input--narrow"
            placeholder={t("score")}
            value={newScore}
            inputMode="decimal"
            onChange={(e) => setNewScore(e.currentTarget.value)}
          />
          <input
            className="rds-input"
            placeholder={t("member")}
            value={newMember}
            onChange={(e) => setNewMember(e.currentTarget.value)}
          />
          <button
            type="button"
            className="btn is-ghost is-compact"
            disabled={
              newMember.trim() === "" ||
              !Number.isFinite(Number.parseFloat(newScore))
            }
            onClick={() => {
              const score = Number.parseFloat(newScore);
              if (!Number.isFinite(score) || newMember.trim() === "") return;
              void onEdit({ kind: "zset-add", score, member: newMember.trim() });
              setNewMember("");
              setNewScore("");
            }}
          >
            <Plus size={10} /> {t("ZADD")}
          </button>
        </div>
      )}
    </div>
  );
}

// ── Stream (read-only) ───────────────────────────────────────────

function StreamView({ preview, total }: { preview: string[]; total: number }) {
  const { t } = useI18n();
  const { query, setQuery, format, setFormat, fmt, match } = useEntryView();
  const rows = preview.map((value, i) => ({ value, i })).filter(({ value }) => match(value));
  return (
    <div className="rds-kv">
      <CollectionToolbar
        query={query}
        onQuery={setQuery}
        format={format}
        onFormat={setFormat}
        loaded={preview.length}
        total={total}
      />
      <div className="rds-kv-head">
        <span>#</span>
        <span>{t("ENTRY")}</span>
      </div>
      {rows.map(({ value, i }) => (
        <div key={i} className="rds-kv-row">
          <span className="rds-kv-field">{i}</span>
          <span className="rds-kv-value">{fmt(value)}</span>
        </div>
      ))}
    </div>
  );
}
