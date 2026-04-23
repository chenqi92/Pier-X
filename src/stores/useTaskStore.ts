import { create } from "zustand";

export type TaskStatus = "running" | "done" | "error";

export type TaskItem = {
  id: number;
  /** Short label shown in the tray. Keep under ~40 chars. */
  label: string;
  /** Optional sub-label; the command / path the task is acting on. */
  detail?: string;
  /** 0..1 when the task reports progress. Undefined = indeterminate. */
  progress?: number;
  status: TaskStatus;
  /** Terminal message set when the task finishes. */
  result?: string;
  startedAt: number;
  finishedAt?: number;
};

type TaskStore = {
  tasks: TaskItem[];
  /** Open the tray popover from the status bar. UI-only state. */
  trayOpen: boolean;
  setTrayOpen: (open: boolean) => void;
  /** Start a new task and return its id. */
  start: (input: { label: string; detail?: string; progress?: number }) => number;
  /** Patch an in-flight task. Silently drops updates for ids that
   *  were already finished — avoids stale callers overwriting
   *  final state. */
  update: (
    id: number,
    patch: Partial<Pick<TaskItem, "label" | "detail" | "progress">>,
  ) => void;
  /** Mark a task done or failed. Captures `finishedAt`. */
  finish: (id: number, outcome: { status: "done" | "error"; result?: string }) => void;
  /** Drop any finished tasks from the list. */
  clearFinished: () => void;
  /** Drop a single task. */
  remove: (id: number) => void;
};

let nextId = 1;

export const useTaskStore = create<TaskStore>((set) => ({
  tasks: [],
  trayOpen: false,

  setTrayOpen: (trayOpen) => set({ trayOpen }),

  start: ({ label, detail, progress }) => {
    const id = nextId++;
    const task: TaskItem = {
      id,
      label,
      detail,
      progress,
      status: "running",
      startedAt: Date.now(),
    };
    set((s) => ({ tasks: [...s.tasks, task] }));
    return id;
  },

  update: (id, patch) => {
    set((s) => ({
      tasks: s.tasks.map((t) =>
        t.id === id && t.status === "running" ? { ...t, ...patch } : t,
      ),
    }));
  },

  finish: (id, { status, result }) => {
    set((s) => ({
      tasks: s.tasks.map((t) =>
        t.id === id
          ? { ...t, status, result, progress: status === "done" ? 1 : t.progress, finishedAt: Date.now() }
          : t,
      ),
    }));
  },

  clearFinished: () => {
    set((s) => ({ tasks: s.tasks.filter((t) => t.status === "running") }));
  },

  remove: (id) => {
    set((s) => ({ tasks: s.tasks.filter((t) => t.id !== id) }));
  },
}));

/**
 * Run an async action as a tracked task. Creates a task, awaits
 * the action, and marks the task done / error based on the
 * outcome. Returns the action's resolved value so callers can
 * chain on it like they would on a plain promise.
 *
 * The `result` string for a successful task defaults to the
 * awaited value (when it's a string) so panels that return a
 * `git` stdout summary get it shown in the tray for free.
 */
export async function withTask<T>(
  label: string,
  action: () => Promise<T>,
  opts?: { detail?: string },
): Promise<T> {
  const store = useTaskStore.getState();
  const id = store.start({ label, detail: opts?.detail });
  try {
    const value = await action();
    store.finish(id, {
      status: "done",
      result: typeof value === "string" ? value.trim() || undefined : undefined,
    });
    return value;
  } catch (error) {
    store.finish(id, { status: "error", result: String(error) });
    throw error;
  }
}
