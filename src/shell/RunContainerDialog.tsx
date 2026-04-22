import { Container, Play, Plus, Trash2, X } from "lucide-react";
import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import IconButton from "../components/IconButton";
import { useDraggableDialog } from "../components/useDraggableDialog";
import { useI18n } from "../i18n/useI18n";
import type { DockerRunOptions } from "../lib/commands";

type Props = {
  open: boolean;
  busy?: boolean;
  defaultImage?: string;
  onClose: () => void;
  onSubmit: (options: DockerRunOptions) => void | Promise<void>;
};

type Pair = { k: string; v: string };

const RESTART_MODES: Array<{ id: string; label: string }> = [
  { id: "", label: "No" },
  { id: "always", label: "Always" },
  { id: "on-failure", label: "On failure" },
  { id: "unless-stopped", label: "Unless stopped" },
];

export default function RunContainerDialog({ open, busy, defaultImage, onClose, onSubmit }: Props) {
  const { t } = useI18n();
  const { dialogStyle, handleProps } = useDraggableDialog(open);
  const [image, setImage] = useState(defaultImage ?? "");
  const [name, setName] = useState("");
  const [ports, setPorts] = useState<Pair[]>([{ k: "", v: "" }]);
  const [envs, setEnvs] = useState<Pair[]>([{ k: "", v: "" }]);
  const [vols, setVols] = useState<Pair[]>([{ k: "", v: "" }]);
  const [restart, setRestart] = useState("");
  const [command, setCommand] = useState("");

  useEffect(() => {
    if (open) {
      setImage(defaultImage ?? "");
      setName("");
      setPorts([{ k: "", v: "" }]);
      setEnvs([{ k: "", v: "" }]);
      setVols([{ k: "", v: "" }]);
      setRestart("");
      setCommand("");
    }
  }, [open, defaultImage]);

  // Close on Esc. We skip while a submission is in flight so the user
  // can't accidentally dismiss the dialog mid-`docker run` and lose
  // track of whether the container got created.
  useEffect(() => {
    if (!open) return;
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape" && !busy) onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, busy, onClose]);

  if (!open) return null;

  const canRun = image.trim().length > 0 && !busy;

  function submit() {
    if (!canRun) return;
    const clean = (arr: Pair[]): [string, string][] =>
      arr
        .filter((p) => p.k.trim() || p.v.trim())
        .map((p) => [p.k, p.v] as [string, string]);
    void onSubmit({
      image: image.trim(),
      name: name.trim(),
      ports: clean(ports),
      env: clean(envs),
      volumes: clean(vols),
      restart,
      command: command.trim(),
    });
  }

  // Portal to <body> so the overlay escapes the right-panel's stacking
  // context and scrim covers the whole window, matching every other app
  // dialog (NewConnection, Settings, DiffDialog).
  return createPortal(
    <div className="cmdp-overlay" onClick={onClose}>
      <div className="dlg dlg--runctr" style={dialogStyle} onClick={(e) => e.stopPropagation()}>
        <div className="dlg-head" {...handleProps}>
          <span className="dlg-title">
            <Play size={12} />
            {t("Run container")}
          </span>
          <div style={{ flex: 1 }} />
          <IconButton variant="mini" onClick={onClose} title={t("Close")}>
            <X size={12} />
          </IconButton>
        </div>
        <div className="dlg-body dlg-body--form">
          <div className="dlg-form">
            <div className="dlg-row">
              <label className="dlg-row-label">{t("Image")} *</label>
              <input
                className="dlg-input"
                placeholder="nginx:1.27-alpine"
                value={image}
                onChange={(e) => setImage(e.currentTarget.value)}
              />
            </div>
            <div className="dlg-row">
              <label className="dlg-row-label">{t("Container name")}</label>
              <input
                className="dlg-input"
                placeholder="e.g. my-app"
                value={name}
                onChange={(e) => setName(e.currentTarget.value)}
              />
            </div>

            <PairGroup
              label={t("Port mappings")}
              leftPlaceholder={t("Host port")}
              rightPlaceholder={t("Container port")}
              separator=":"
              addLabel={t("Add port")}
              pairs={ports}
              setPairs={setPorts}
              t={t}
            />

            <PairGroup
              label={t("Environment variables")}
              leftPlaceholder="KEY"
              rightPlaceholder="value"
              separator="="
              addLabel={t("Add variable")}
              pairs={envs}
              setPairs={setEnvs}
              t={t}
            />

            <PairGroup
              label={t("Volume mounts")}
              leftPlaceholder={t("Host path")}
              rightPlaceholder={t("Container path")}
              separator="→"
              addLabel={t("Add volume")}
              pairs={vols}
              setPairs={setVols}
              t={t}
            />

            <div className="dlg-row">
              <label className="dlg-row-label">{t("Restart policy")}</label>
              <div className="dlg-opts" role="radiogroup">
                {RESTART_MODES.map((m) => (
                  <button
                    key={m.id || "no"}
                    type="button"
                    role="radio"
                    aria-checked={restart === m.id}
                    className={"dlg-opt" + (restart === m.id ? " active" : "")}
                    onClick={() => setRestart(m.id)}
                  >
                    {t(m.label)}
                  </button>
                ))}
              </div>
            </div>

            <div className="dlg-row">
              <label className="dlg-row-label">{t("Command")}</label>
              <input
                className="dlg-input"
                placeholder="e.g. /bin/sh"
                value={command}
                onChange={(e) => setCommand(e.currentTarget.value)}
              />
            </div>
          </div>
        </div>
        <div className="dlg-foot">
          <div style={{ flex: 1 }} />
          <button type="button" className="gb-btn" onClick={onClose}>{t("Cancel")}</button>
          <button type="button" className="gb-btn primary" disabled={!canRun} onClick={submit}>
            <Container size={11} /> {t("Create & run")}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}

function PairGroup({
  label,
  leftPlaceholder,
  rightPlaceholder,
  separator,
  addLabel,
  pairs,
  setPairs,
  t,
}: {
  label: string;
  leftPlaceholder: string;
  rightPlaceholder: string;
  separator: string;
  addLabel: string;
  pairs: Pair[];
  setPairs: (p: Pair[]) => void;
  t: (s: string) => string;
}) {
  return (
    <div className="dlg-row">
      {/* Pair stacks are taller than a single input, so pin the label
          to the top instead of letting `.dlg-row { align-items: center }`
          float it to the middle of the stack. */}
      <label
        className="dlg-row-label"
        style={{ alignSelf: "start", paddingTop: "var(--sp-1-5)" }}
      >
        {label}
      </label>
      <div className="run-pair-stack">
        {pairs.map((p, i) => (
          <div key={i} className="run-pair">
            <input
              className="dlg-input"
              placeholder={leftPlaceholder}
              value={p.k}
              onChange={(e) => {
                const next = pairs.slice();
                next[i] = { ...next[i], k: e.currentTarget.value };
                setPairs(next);
              }}
            />
            <span className="run-pair-sep mono">{separator}</span>
            <input
              className="dlg-input"
              placeholder={rightPlaceholder}
              value={p.v}
              onChange={(e) => {
                const next = pairs.slice();
                next[i] = { ...next[i], v: e.currentTarget.value };
                setPairs(next);
              }}
            />
            {pairs.length > 1 && (
              <button
                type="button"
                className="mini-btn is-destructive"
                title={t("Remove")}
                onClick={() => setPairs(pairs.filter((_, j) => j !== i))}
              >
                <Trash2 size={11} />
              </button>
            )}
          </div>
        ))}
        <button
          type="button"
          className="run-pair-add"
          onClick={() => setPairs([...pairs, { k: "", v: "" }])}
        >
          <Plus size={11} /> {addLabel}
        </button>
      </div>
    </div>
  );
}
