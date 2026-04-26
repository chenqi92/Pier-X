import { Check, Edit, Plus, Save, Trash2, X } from "lucide-react";
import RedisIcon from "../icons/RedisIcon";
import { useEffect, useRef, useState } from "react";

import { useI18n } from "../../i18n/useI18n";
import RedisTypeBadge from "./RedisTypeBadge";
import type { RedisKeyView } from "../../lib/types";

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
  | { kind: "list-rem"; value: string; count: number }
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

      {isString ? (
        <StringView
          preview={details.preview}
          onEdit={editEnabled ? onEdit : undefined}
        />
      ) : isHash ? (
        <HashView
          preview={details.preview}
          onEdit={editEnabled ? onEdit : undefined}
        />
      ) : isList ? (
        <ListView
          preview={details.preview}
          onEdit={editEnabled ? onEdit : undefined}
        />
      ) : isSet ? (
        <SetView
          preview={details.preview}
          onEdit={editEnabled ? onEdit : undefined}
        />
      ) : isZset ? (
        <ZsetView
          preview={details.preview}
          onEdit={editEnabled ? onEdit : undefined}
        />
      ) : (
        // Stream — keep read-only for now; XADD's field/value
        // pairs need a more elaborate form than this view.
        <StreamView preview={details.preview} />
      )}

      {details.previewTruncated && (
        <div className="rds-truncated-note">
          {t("Preview truncated — the value continues beyond what's shown.")}
        </div>
      )}
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
  // Reset the draft whenever the source value changes — typically
  // after a successful Save the panel re-fetches the key and we get
  // a fresh `preview` here.
  useEffect(() => {
    setDraft(original);
    setEditing(false);
  }, [original]);

  if (!editing) {
    return (
      <div className="rds-value">
        <div className="rds-value-head">
          {t("VALUE")}
          {onEdit && (
            <button
              type="button"
              className="mini-button mini-button--ghost"
              style={{ marginLeft: "auto" }}
              onClick={() => setEditing(true)}
              title={t("Edit value")}
            >
              <Edit size={9} /> {t("Edit")}
            </button>
          )}
        </div>
        <div className="rds-value-body">{original}</div>
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
  onEdit,
}: {
  preview: string[];
  onEdit?: (op: RedisEdit) => Promise<void>;
}) {
  const { t } = useI18n();
  const pairs: [string, string][] = [];
  for (let i = 0; i < preview.length; i += 2) {
    pairs.push([preview[i], preview[i + 1] ?? ""]);
  }
  const [editingField, setEditingField] = useState<string | null>(null);
  const [editDraft, setEditDraft] = useState("");
  const [newField, setNewField] = useState("");
  const [newValue, setNewValue] = useState("");

  return (
    <div className="rds-kv">
      <div className="rds-kv-head">
        <span>{t("FIELD")}</span>
        <span>{t("VALUE")}</span>
        {onEdit && <span className="rds-kv-actions" aria-hidden />}
      </div>
      {pairs.map(([field, value], i) => {
        const isEditing = editingField === field;
        return (
          <div key={`${field}-${i}`} className="rds-kv-row">
            <span className="rds-kv-field">{field}</span>
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
              <span className="rds-kv-value">{value}</span>
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
                      onClick={() => {
                        if (
                          window.confirm(t("Delete field {field}?", { field }))
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
  onEdit,
}: {
  preview: string[];
  onEdit?: (op: RedisEdit) => Promise<void>;
}) {
  const { t } = useI18n();
  const [editingIdx, setEditingIdx] = useState<number | null>(null);
  const [editDraft, setEditDraft] = useState("");
  const [newValue, setNewValue] = useState("");

  return (
    <div className="rds-kv">
      <div className="rds-kv-head">
        <span>#</span>
        <span>{t("ELEMENT")}</span>
        {onEdit && <span className="rds-kv-actions" aria-hidden />}
      </div>
      {preview.map((value, i) => {
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
              <span className="rds-kv-value">{value}</span>
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
                      onClick={() => {
                        if (window.confirm(t("Remove the first occurrence of this element (LREM 1)?"))) {
                          void onEdit({ kind: "list-rem", value, count: 1 });
                        }
                      }}
                      title={t("Remove (LREM)")}
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
  onEdit,
}: {
  preview: string[];
  onEdit?: (op: RedisEdit) => Promise<void>;
}) {
  const { t } = useI18n();
  const [newMember, setNewMember] = useState("");

  return (
    <div className="rds-kv">
      <div className="rds-kv-head">
        <span>#</span>
        <span>{t("MEMBER")}</span>
        {onEdit && <span className="rds-kv-actions" aria-hidden />}
      </div>
      {preview.map((member, i) => (
        <div key={`${member}-${i}`} className="rds-kv-row">
          <span className="rds-kv-field">{i}</span>
          <span className="rds-kv-value">{member}</span>
          {onEdit && (
            <span className="rds-kv-actions">
              <button
                type="button"
                className="mini-button mini-button--ghost"
                onClick={() => {
                  if (window.confirm(t("Remove member {member}?", { member }))) {
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
  onEdit,
}: {
  preview: string[];
  onEdit?: (op: RedisEdit) => Promise<void>;
}) {
  const { t } = useI18n();
  const entries = preview.map(parseZsetEntry);
  const [editingMember, setEditingMember] = useState<string | null>(null);
  const [editDraft, setEditDraft] = useState("");
  const [newMember, setNewMember] = useState("");
  const [newScore, setNewScore] = useState("");

  return (
    <div className="rds-kv">
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
            <span className="rds-kv-value">{member}</span>
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
                      onClick={() => {
                        if (window.confirm(t("Remove member {member}?", { member }))) {
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

function StreamView({ preview }: { preview: string[] }) {
  const { t } = useI18n();
  return (
    <div className="rds-kv">
      <div className="rds-kv-head">
        <span>#</span>
        <span>{t("ENTRY")}</span>
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
