import {
  ArrowRight,
  Check,
  KeyRound,
  Link2,
  Pencil,
  Plus,
  Save,
  Trash2,
  X,
  Zap,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from "react";

import { useI18n } from "../../i18n/useI18n";
import DbStubView from "./DbStubView";
import type { DdlMutation } from "./dbColumnRules";

export type DbStructureColumn = {
  name: string;
  type: string;
  /** Whether the column is part of the primary key. */
  pk: boolean;
  /** Whether the column allows NULL. */
  nullable: boolean;
  /** Engine-specific extra column for the "Key" position (e.g. MySQL's
   *  `MUL`/`UNI` index hint). Free-form text — empty / undefined renders as a dash. */
  keyHint?: string;
  /** Default value, if any. Empty/undefined renders as a dash. */
  defaultValue?: string | null;
  /** Free-form extra (MySQL `EXTRA`, PG attoptions). Optional. */
  extra?: string;
};

/** Index row rendered in the Structure tab's Indexes section.
 *  `unique` controls the badge tint; `kind` shows up as the
 *  small uppercase suffix (BTREE / HASH / GIN / …). */
export type DbStructureIndex = {
  name: string;
  columns: string[];
  unique: boolean;
  kind: string;
};

/** Trigger row rendered in the Structure tab's Triggers
 *  section. SQLite-only for now — `event` is e.g. "BEFORE INSERT"
 *  or "AFTER UPDATE", and `sql` is the full CREATE TRIGGER
 *  statement (shown on hover via `title`). */
export type DbStructureTrigger = {
  name: string;
  event: string;
  sql: string;
};

/** Foreign-key row rendered in the Structure tab's Foreign keys
 *  section. Composite keys pair `columns` with `refColumns` by
 *  index — the renderer joins them with " · " for display. */
export type DbStructureForeignKey = {
  name: string;
  columns: string[];
  refSchema: string;
  refTable: string;
  refColumns: string[];
  onUpdate: string;
  onDelete: string;
};

/** Locally-tracked pending new column. The user fills in the form,
 *  then Commit packages it into an `addColumn` DDL mutation. */
type PendingAdd = {
  /** Stable id for React keying — not part of the DDL. */
  id: string;
  name: string;
  type: string;
  nullable: boolean;
  /** `null` = leave DEFAULT off entirely. The form treats empty
   *  string the same as `null` so casual adds don't accidentally
   *  emit `DEFAULT ''`. */
  defaultValue: string | null;
};

type Props = {
  columns: DbStructureColumn[];
  /** Color the type cells with the engine accent (e.g. `var(--svc-mysql)`). */
  typeAccentVar: string;
  /** Optional Indexes section. Empty / undefined hides the
   *  section entirely so older callers (or tables with no
   *  indexes) don't show an empty header. */
  indexes?: DbStructureIndex[];
  /** Optional Foreign keys section. Same omit-when-empty
   *  semantics as `indexes`. */
  foreignKeys?: DbStructureForeignKey[];
  /** Optional Triggers section (SQLite). Same omit-when-empty
   *  semantics as `indexes`. */
  triggers?: DbStructureTrigger[];
  /** Optional footer note. Use it for "this is what we still
   *  don't show" callouts; the indexes / FK gaps are now
   *  closed so the note typically goes blank. */
  footnote?: ReactNode;

  /** When true, renders inline edit affordances (rename, drop,
   *  add column) and the commit footer. Defaults to false so
   *  the structure tab stays read-only unless the panel opts
   *  in by passing both `editable` and `onCommit`. */
  editable?: boolean;
  /** Wire-up for the commit footer. Receives the staged DDL
   *  mutation list; the panel translates each into engine SQL
   *  via [`ddlToSql`] and ships it through the `*_execute`
   *  command. The view clears its pending state on resolve. */
  onCommit?: (mutations: DdlMutation[]) => Promise<void>;
  /** True while the panel's commit Promise is in flight — disables
   *  the buttons so the user can't double-fire. */
  committing?: boolean;
};

export default function DbStructureView({
  columns,
  typeAccentVar,
  indexes,
  foreignKeys,
  triggers,
  footnote,
  editable = false,
  onCommit,
  committing = false,
}: Props) {
  const { t } = useI18n();

  // ── Pending DDL state. Reset whenever the source columns swap
  // (typically a table-switch or a successful commit re-fetch). ──
  const [pendingDrops, setPendingDrops] = useState<Set<string>>(new Set());
  const [pendingRenames, setPendingRenames] = useState<Map<string, string>>(new Map());
  const [pendingAdds, setPendingAdds] = useState<PendingAdd[]>([]);
  const [editingRename, setEditingRename] = useState<string | null>(null);
  const [renameDraft, setRenameDraft] = useState("");

  useEffect(() => {
    setPendingDrops(new Set());
    setPendingRenames(new Map());
    setPendingAdds([]);
    setEditingRename(null);
    setRenameDraft("");
    // Reset on column-shape change. Reference equality on the
    // columns array would be too aggressive (we get a fresh array
    // on every render), so key off the JSON-stringified column
    // names — stable as long as the schema doesn't change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [columns.map((c) => c.name).join("|")]);

  const editableEnabled = editable && !!onCommit;
  const pendingCount =
    pendingDrops.size + pendingRenames.size + pendingAdds.length;

  function startRename(col: DbStructureColumn) {
    if (!editableEnabled || pendingDrops.has(col.name)) return;
    setEditingRename(col.name);
    setRenameDraft(pendingRenames.get(col.name) ?? col.name);
  }

  function commitRename(originalName: string, draft: string) {
    setEditingRename(null);
    const next = draft.trim();
    if (!next || next === originalName) {
      // Either invalid or unchanged — drop any pending rename for
      // this column so a no-op edit cleans up after itself.
      setPendingRenames((prev) => {
        if (!prev.has(originalName)) return prev;
        const m = new Map(prev);
        m.delete(originalName);
        return m;
      });
      return;
    }
    setPendingRenames((prev) => {
      const m = new Map(prev);
      m.set(originalName, next);
      return m;
    });
  }

  function cancelRename() {
    setEditingRename(null);
    setRenameDraft("");
  }

  function toggleDrop(col: DbStructureColumn) {
    if (!editableEnabled) return;
    setPendingDrops((prev) => {
      const next = new Set(prev);
      if (next.has(col.name)) next.delete(col.name);
      else next.add(col.name);
      return next;
    });
    // Dropping a column that was being renamed → discard the rename.
    if (pendingRenames.has(col.name)) {
      setPendingRenames((prev) => {
        const m = new Map(prev);
        m.delete(col.name);
        return m;
      });
    }
    if (editingRename === col.name) cancelRename();
  }

  function startAdd() {
    if (!editableEnabled) return;
    setPendingAdds((prev) => [
      ...prev,
      {
        id: `pending-${Date.now()}-${prev.length}`,
        name: "",
        type: "",
        nullable: true,
        defaultValue: null,
      },
    ]);
  }

  function patchAdd(id: string, patch: Partial<PendingAdd>) {
    setPendingAdds((prev) => prev.map((a) => (a.id === id ? { ...a, ...patch } : a)));
  }

  function removeAdd(id: string) {
    setPendingAdds((prev) => prev.filter((a) => a.id !== id));
  }

  function discardAll() {
    setPendingDrops(new Set());
    setPendingRenames(new Map());
    setPendingAdds([]);
    setEditingRename(null);
    setRenameDraft("");
  }

  /** Validate + assemble. Validation is intentionally light — the
   *  backend is the source of truth for type / identifier rules.
   *  We only block obviously-broken submissions (empty name / type)
   *  and flag duplicates within the pending batch. Returns `null`
   *  on success or a localized error message. */
  function validateAndCollect(): { ok: true; muts: DdlMutation[] } | { ok: false; msg: string } {
    const muts: DdlMutation[] = [];

    // Drops first — they don't conflict with each other and they
    // free up the namespace for adds in the same batch.
    for (const name of pendingDrops) {
      muts.push({ kind: "dropColumn", name });
    }

    // Renames — skip any column that is also being dropped (drops win).
    for (const [oldName, newName] of pendingRenames) {
      if (pendingDrops.has(oldName)) continue;
      const trimmed = newName.trim();
      if (!trimmed) {
        return { ok: false, msg: t("Rename target is empty.") };
      }
      muts.push({ kind: "renameColumn", oldName, newName: trimmed });
    }

    // Adds — name + type required.
    const addNames = new Set<string>();
    for (const add of pendingAdds) {
      const name = add.name.trim();
      const type = add.type.trim();
      if (!name) return { ok: false, msg: t("New column name is empty.") };
      if (!type) {
        return { ok: false, msg: t("Type for column \"{name}\" is empty.", { name }) };
      }
      if (addNames.has(name)) {
        return { ok: false, msg: t("Duplicate new column \"{name}\".", { name }) };
      }
      addNames.add(name);
      const def = add.defaultValue;
      muts.push({
        kind: "addColumn",
        name,
        type,
        nullable: add.nullable,
        defaultValue: def === null || def === "" ? null : def,
      });
    }

    return { ok: true, muts };
  }

  async function onCommitClick() {
    if (!onCommit) return;
    const result = validateAndCollect();
    if (!result.ok) {
      // Use a window.alert for the validation gate — the structure
      // tab doesn't host an inline error banner today and the
      // panel-level banner would require a callback prop. Validation
      // failures should be rare (the form already prevents most).
      window.alert(result.msg);
      return;
    }
    if (result.muts.length === 0) return;
    await onCommit(result.muts);
    // Caller is expected to re-fetch and remount this view (column
    // identity changes) — but if it doesn't, clear local state so
    // the user isn't left with stale "pending" markers.
    discardAll();
  }

  if (columns.length === 0 && pendingAdds.length === 0) {
    return <DbStubView title={t("No table selected")} />;
  }

  const typeStyle: CSSProperties = { color: typeAccentVar };
  const renamedDisplayName = (col: DbStructureColumn): string =>
    pendingRenames.get(col.name) ?? col.name;

  return (
    <div className="db2-structure">
      <div className="db2-structure__head">
        <span className="db2-structure__title">{t("Columns")}</span>
        <span className="db2-structure__count">
          {columns.length + pendingAdds.length}
        </span>
        {editableEnabled && (
          <span className="db2-structure__head-actions">
            <button
              type="button"
              className="btn is-ghost is-compact"
              onClick={startAdd}
              disabled={committing}
              title={t("Add column")}
            >
              <Plus size={10} /> {t("Add column")}
            </button>
          </span>
        )}
      </div>
      <div className="db2-structure__scroll">
        <table className="rg-table db2-structure__table">
          <thead>
            <tr>
              <th className="db2-structure__th-pk" />
              <th>
                <div className="rg-th-body">
                  <span className="rg-th-name">{t("Name")}</span>
                </div>
              </th>
              <th>
                <div className="rg-th-body">
                  <span className="rg-th-name">{t("Type")}</span>
                </div>
              </th>
              <th>
                <div className="rg-th-body">
                  <span className="rg-th-name">{t("Null")}</span>
                </div>
              </th>
              <th>
                <div className="rg-th-body">
                  <span className="rg-th-name">{t("Default")}</span>
                </div>
              </th>
              <th>
                <div className="rg-th-body">
                  <span className="rg-th-name">{t("Extra")}</span>
                </div>
              </th>
              {editableEnabled && <th className="db2-structure__th-acts" />}
            </tr>
          </thead>
          <tbody>
            {pendingAdds.map((add) => (
              <PendingAddRow
                key={add.id}
                add={add}
                onChange={(patch) => patchAdd(add.id, patch)}
                onRemove={() => removeAdd(add.id)}
                t={t}
              />
            ))}
            {columns.map((col) => {
              const def = col.defaultValue;
              const defText =
                def === null || def === undefined || def === "" ? "—" : def;
              const extraBits = [col.keyHint, col.extra].filter((s): s is string => !!s && s.trim() !== "");
              const dropped = pendingDrops.has(col.name);
              const renamed = pendingRenames.has(col.name);
              const display = renamedDisplayName(col);
              const rowClass =
                "rg-row" +
                (dropped ? " db2-structure__row--dropped" : "") +
                (renamed && !dropped ? " db2-structure__row--renamed" : "");
              return (
                <tr key={col.name} className={rowClass}>
                  <td className="rg-td db2-structure__td-pk">
                    {col.pk && (
                      <span
                        className="rg-pk"
                        title={t("Primary key")}
                        aria-label={t("Primary key")}
                      >
                        <KeyRound size={11} />
                      </span>
                    )}
                  </td>
                  <td className="rg-td">
                    {editingRename === col.name ? (
                      <RenameEditor
                        initial={renameDraft}
                        onCommit={(v) => commitRename(col.name, v)}
                        onCancel={cancelRename}
                      />
                    ) : (
                      <span className="db2-structure__name">
                        {display}
                        {renamed && !dropped && (
                          <span
                            className="db2-structure__renamed-tag"
                            title={t("Renamed from {name}", { name: col.name })}
                          >
                            ←{col.name}
                          </span>
                        )}
                      </span>
                    )}
                  </td>
                  <td className="rg-td" style={typeStyle}>
                    {col.type}
                  </td>
                  <td className="rg-td">{col.nullable ? t("YES") : t("NO")}</td>
                  <td className="rg-td">
                    {def === null ? <span className="rg-null">NULL</span> : defText}
                  </td>
                  <td className="rg-td">
                    {extraBits.length > 0 ? extraBits.join(" · ") : "—"}
                  </td>
                  {editableEnabled && (
                    <td className="rg-td-acts" onClick={(e) => e.stopPropagation()}>
                      {dropped ? (
                        <button
                          type="button"
                          className="mini-button mini-button--ghost"
                          onClick={() => toggleDrop(col)}
                          title={t("Undo drop")}
                          disabled={committing}
                        >
                          <X size={10} />
                        </button>
                      ) : (
                        <>
                          <button
                            type="button"
                            className="mini-button mini-button--ghost"
                            onClick={() => startRename(col)}
                            title={t("Rename column")}
                            disabled={committing}
                          >
                            <Pencil size={10} />
                          </button>
                          <button
                            type="button"
                            className="mini-button mini-button--ghost"
                            onClick={() => toggleDrop(col)}
                            title={t("Drop column")}
                            disabled={committing || col.pk}
                          >
                            <Trash2 size={10} />
                          </button>
                        </>
                      )}
                    </td>
                  )}
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>

      {editableEnabled && pendingCount > 0 && (
        <div className="db2-structure__commit">
          <span className="db2-structure__pending-stat">
            {t("{n} structure changes pending", { n: pendingCount })}
          </span>
          <span className="db2-structure__pending-spacer" />
          <button
            type="button"
            className="btn is-ghost is-compact"
            onClick={discardAll}
            disabled={committing}
          >
            <X size={10} /> {t("Discard")}
          </button>
          <button
            type="button"
            className="btn is-primary is-compact"
            onClick={() => void onCommitClick()}
            disabled={committing}
          >
            <Save size={10} />{" "}
            {committing
              ? t("Committing...")
              : t("Commit {n} changes", { n: pendingCount })}
          </button>
        </div>
      )}

      {indexes && indexes.length > 0 && (
        <div className="db2-structure__section">
          <div className="db2-structure__head">
            <span className="db2-structure__title">{t("Indexes")}</span>
            <span className="db2-structure__count">{indexes.length}</span>
          </div>
          <div className="db2-structure__list">
            {indexes.map((idx) => (
              <div key={idx.name} className="db2-structure__row">
                <span className="db2-structure__row-icon" aria-hidden>
                  <KeyRound size={11} />
                </span>
                <span className="db2-structure__row-name">{idx.name}</span>
                <span className="db2-structure__row-cols">
                  {idx.columns.join(", ")}
                </span>
                <span className="db2-structure__row-meta">
                  {idx.unique ? t("UNIQUE") : t("INDEX")}
                  {idx.kind ? ` · ${idx.kind.toUpperCase()}` : ""}
                </span>
              </div>
            ))}
          </div>
        </div>
      )}

      {foreignKeys && foreignKeys.length > 0 && (
        <div className="db2-structure__section">
          <div className="db2-structure__head">
            <span className="db2-structure__title">{t("Foreign keys")}</span>
            <span className="db2-structure__count">{foreignKeys.length}</span>
          </div>
          <div className="db2-structure__list">
            {foreignKeys.map((fk) => {
              const target = fk.refSchema
                ? `${fk.refSchema}.${fk.refTable}`
                : fk.refTable;
              const cascade =
                fk.onUpdate && fk.onDelete
                  ? t("ON UPDATE {u} · ON DELETE {d}", {
                      u: fk.onUpdate,
                      d: fk.onDelete,
                    })
                  : "";
              return (
                <div key={fk.name} className="db2-structure__row">
                  <span className="db2-structure__row-icon" aria-hidden>
                    <Link2 size={11} />
                  </span>
                  <span className="db2-structure__row-name">{fk.name}</span>
                  <span className="db2-structure__row-cols">
                    {fk.columns.join(", ")}
                    <ArrowRight size={9} className="db2-structure__row-arrow" />
                    <span className="db2-structure__row-target">{target}</span>
                    {fk.refColumns.length > 0 && (
                      <>
                        {" · "}
                        {fk.refColumns.join(", ")}
                      </>
                    )}
                  </span>
                  {cascade && (
                    <span className="db2-structure__row-meta">{cascade}</span>
                  )}
                </div>
              );
            })}
          </div>
        </div>
      )}

      {triggers && triggers.length > 0 && (
        <div className="db2-structure__section">
          <div className="db2-structure__head">
            <span className="db2-structure__title">{t("Triggers")}</span>
            <span className="db2-structure__count">{triggers.length}</span>
          </div>
          <div className="db2-structure__list">
            {triggers.map((tr) => (
              <div
                key={tr.name}
                className="db2-structure__row"
                title={tr.sql}
              >
                <span className="db2-structure__row-icon" aria-hidden>
                  <Zap size={11} />
                </span>
                <span className="db2-structure__row-name">{tr.name}</span>
                <span className="db2-structure__row-cols">{tr.event}</span>
                <span className="db2-structure__row-meta" />
              </div>
            ))}
          </div>
        </div>
      )}

      {footnote && <div className="db2-structure__footnote">{footnote}</div>}
    </div>
  );
}

/** Inline rename editor — autofocuses, commits on blur or Enter,
 *  cancels on Escape. Mirrors `CellEditor` in `DbResultGrid`. */
function RenameEditor({
  initial,
  onCommit,
  onCancel,
}: {
  initial: string;
  onCommit: (v: string) => void;
  onCancel: () => void;
}) {
  const ref = useRef<HTMLInputElement | null>(null);
  const [val, setVal] = useState(initial);
  useEffect(() => {
    ref.current?.focus();
    ref.current?.select();
  }, []);
  return (
    <input
      ref={ref}
      className="rg-td-input"
      value={val}
      onChange={(e) => setVal(e.currentTarget.value)}
      onBlur={() => onCommit(val)}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          (e.currentTarget as HTMLInputElement).blur();
        } else if (e.key === "Escape") {
          e.preventDefault();
          onCancel();
        }
      }}
    />
  );
}

/** New-column row — three inputs (name / type / default) plus a NULL
 *  toggle. Lives at the top of the columns table while the user is
 *  staging the add; removed from the staging list (and thus from the
 *  view) when committed or discarded. */
function PendingAddRow({
  add,
  onChange,
  onRemove,
  t,
}: {
  add: PendingAdd;
  onChange: (patch: Partial<PendingAdd>) => void;
  onRemove: () => void;
  t: (s: string, vars?: Record<string, string | number>) => string;
}) {
  // Default-toggle UX: the user types a default value to set one,
  // clears it back to null to drop the DEFAULT clause. We store "" as
  // null so an empty input doesn't accidentally emit `DEFAULT ''`.
  const defaultDisplay = useMemo(
    () => (add.defaultValue ?? ""),
    [add.defaultValue],
  );

  return (
    <tr className="rg-row db2-structure__row--new">
      <td className="rg-td db2-structure__td-pk">
        <span className="rg-row-badge">{t("NEW")}</span>
      </td>
      <td className="rg-td">
        <input
          className="rg-td-input"
          placeholder={t("column_name")}
          value={add.name}
          onChange={(e) => onChange({ name: e.currentTarget.value })}
        />
      </td>
      <td className="rg-td">
        <input
          className="rg-td-input"
          placeholder={t("e.g. VARCHAR(255)")}
          value={add.type}
          onChange={(e) => onChange({ type: e.currentTarget.value })}
        />
      </td>
      <td className="rg-td">
        <select
          className="rg-td-input"
          value={add.nullable ? "yes" : "no"}
          onChange={(e) => onChange({ nullable: e.currentTarget.value === "yes" })}
        >
          <option value="yes">{t("YES")}</option>
          <option value="no">{t("NO")}</option>
        </select>
      </td>
      <td className="rg-td">
        <input
          className="rg-td-input"
          placeholder={t("(none)")}
          value={defaultDisplay}
          onChange={(e) => {
            const next = e.currentTarget.value;
            onChange({ defaultValue: next === "" ? null : next });
          }}
        />
      </td>
      <td className="rg-td">—</td>
      <td className="rg-td-acts" onClick={(e) => e.stopPropagation()}>
        <button
          type="button"
          className="mini-button mini-button--ghost"
          onClick={onRemove}
          title={t("Discard insert")}
        >
          <X size={10} />
        </button>
        <span className="rg-row-staged" title={t("Staged for commit")}>
          <Check size={10} />
        </span>
      </td>
    </tr>
  );
}
