import { CheckCircle2, CircleAlert, Loader2, Trash2, X } from "lucide-react";
import { useI18n } from "../i18n/useI18n";
import { useTaskStore, type TaskItem } from "../stores/useTaskStore";

/**
 * Floating popover anchored above the status bar that lists
 * every in-flight and recently-finished task. Users open it
 * by clicking the task-count badge in the status bar. Stays
 * hidden when empty + closed to avoid persistent UI weight.
 */
export default function TaskTray() {
  const { t } = useI18n();
  const tasks = useTaskStore((s) => s.tasks);
  const open = useTaskStore((s) => s.trayOpen);
  const setOpen = useTaskStore((s) => s.setTrayOpen);
  const clearFinished = useTaskStore((s) => s.clearFinished);
  const remove = useTaskStore((s) => s.remove);

  if (!open) return null;

  const sorted = [...tasks].sort((a, b) => {
    // Running first, then newest finished
    if (a.status === "running" && b.status !== "running") return -1;
    if (b.status === "running" && a.status !== "running") return 1;
    return b.startedAt - a.startedAt;
  });
  const finishedCount = tasks.filter((t) => t.status !== "running").length;

  return (
    <div className="cmdp-overlay" style={{ background: "transparent" }} onClick={() => setOpen(false)}>
      <div
        onClick={(e) => e.stopPropagation()}
        style={{
          position: "fixed",
          right: 16,
          bottom: 36,
          width: 380,
          maxHeight: "60vh",
          display: "flex",
          flexDirection: "column",
          background: "var(--elev)",
          border: "1px solid var(--line)",
          borderRadius: "var(--radius-sm)",
          boxShadow: "var(--shadow-popover)",
          zIndex: 800,
          overflow: "hidden",
        }}
      >
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: "var(--sp-2)",
            padding: "var(--sp-2) var(--sp-3)",
            borderBottom: "1px solid var(--line)",
            background: "var(--surface)",
          }}
        >
          <strong>{t("Tasks")}</strong>
          <span className="settings__badge">{tasks.length}</span>
          <div style={{ flex: 1 }} />
          {finishedCount > 0 && (
            <button className="mini-button" onClick={clearFinished} type="button">
              <Trash2 size={11} />
              {t("Clear completed")}
            </button>
          )}
          <button className="mini-button" onClick={() => setOpen(false)} type="button">
            <X size={11} />
          </button>
        </div>
        <div style={{ overflowY: "auto", padding: "var(--sp-2)", display: "flex", flexDirection: "column", gap: "var(--sp-1)" }}>
          {sorted.length === 0 ? (
            <div className="empty-note">{t("No tasks yet.")}</div>
          ) : (
            sorted.map((task) => <TaskRow key={task.id} task={task} onRemove={() => remove(task.id)} />)
          )}
        </div>
      </div>
    </div>
  );
}

function TaskRow({ task, onRemove }: { task: TaskItem; onRemove: () => void }) {
  const { t } = useI18n();
  const icon =
    task.status === "running" ? (
      <Loader2 size={13} className="ftp-spin" color="var(--accent)" />
    ) : task.status === "done" ? (
      <CheckCircle2 size={13} color="var(--pos)" />
    ) : (
      <CircleAlert size={13} color="var(--neg)" />
    );
  const pct =
    typeof task.progress === "number" ? Math.max(0, Math.min(1, task.progress)) : null;

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: 4,
        padding: "var(--sp-2)",
        border: "1px solid var(--line)",
        borderRadius: "var(--radius-xs)",
        background: "var(--surface-2)",
      }}
    >
      <div style={{ display: "flex", alignItems: "center", gap: "var(--sp-2)" }}>
        {icon}
        <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
          {task.label}
        </span>
        {task.status !== "running" && (
          <button
            className="mini-button"
            onClick={onRemove}
            type="button"
            style={{ padding: 2 }}
            title={t("Dismiss")}
          >
            <X size={10} />
          </button>
        )}
      </div>
      {task.detail && (
        <div style={{ fontSize: "var(--ui-fs-sm)", color: "var(--muted)", fontFamily: "var(--mono)" }}>
          {task.detail}
        </div>
      )}
      {task.status === "running" && pct !== null && (
        <div style={{ height: 3, background: "var(--line)", borderRadius: 2, overflow: "hidden" }}>
          <div style={{ width: `${pct * 100}%`, height: "100%", background: "var(--accent)", transition: "width 120ms" }} />
        </div>
      )}
      {task.result && task.status !== "running" && (
        <div
          style={{
            fontSize: "var(--ui-fs-sm)",
            color: task.status === "error" ? "var(--neg)" : "var(--muted)",
            wordBreak: "break-word",
          }}
        >
          {task.result}
        </div>
      )}
    </div>
  );
}
