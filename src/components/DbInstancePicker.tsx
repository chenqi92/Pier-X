import { Plus, RefreshCw, Server, Star, Trash2 } from "lucide-react";
import DockerIcon from "./icons/DockerIcon";
import { useEffect, useMemo, useState } from "react";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";
import * as cmd from "../lib/commands";
import type { DbCredential, DbKind, DetectedDbInstance, TabState } from "../lib/types";
import { effectiveSshTarget } from "../lib/types";
import { useConnectionStore } from "../stores/useConnectionStore";
import { useDetectedServicesStore } from "../stores/useDetectedServicesStore";
import ConfirmDialog from "./ConfirmDialog";

type DetectedKind = "mysql" | "postgres" | "redis";

type Props = {
  tab: TabState;
  kind: Extract<DbKind, "mysql" | "postgres" | "redis">;
  /** User clicked a saved credential → activate + (re)open tunnel. */
  onActivate: (cred: DbCredential) => void;
  /** User adopted a detection result → open the Add dialog pre-filled. */
  onAdopt: (detected: DetectedDbInstance) => void;
  /** User clicked "+ Add" → open the Add dialog blank. */
  onAddNew: () => void;
  /** User deleted a saved credential. */
  onDeleted?: (cred: DbCredential) => void;
};

/** Stale window for cached detection results. */
const DETECTION_TTL_MS = 60_000;

/**
 * Single-panel, top-mounted pill strip that lets the user switch
 * between saved DB credentials and detected instances without
 * leaving the current panel. Mirrors the "detect + pre-fill"
 * pattern from the service chips in the left sidebar.
 */
export default function DbInstancePicker({
  tab,
  kind,
  onActivate,
  onAdopt,
  onAddNew,
  onDeleted,
}: Props) {
  const { t } = useI18n();
  const formatError = (e: unknown) => localizeError(e, t);

  const sshTarget = effectiveSshTarget(tab);
  const savedIndex = sshTarget?.savedConnectionIndex ?? null;

  const connection = useConnectionStore((s) =>
    savedIndex !== null ? s.connections.find((c) => c.index === savedIndex) ?? null : null,
  );
  const refreshConnections = useConnectionStore((s) => s.refresh);

  const savedForKind = useMemo<DbCredential[]>(
    () => (connection?.databases ?? []).filter((c) => c.kind === kind),
    [connection, kind],
  );

  const instancesEntry = useDetectedServicesStore((s) => s.instancesByTab[tab.id]);
  const setPending = useDetectedServicesStore((s) => s.setDbInstancesPending);
  const setInstances = useDetectedServicesStore((s) => s.setDbInstances);
  const setError = useDetectedServicesStore((s) => s.setDbInstancesError);

  // Pending delete target + error banner state for the themed
  // ConfirmDialog that replaced the blocking window.confirm /
  // window.alert calls.
  const [deleteTarget, setDeleteTarget] = useState<DbCredential | null>(null);
  const [deleteError, setDeleteError] = useState("");

  const detectedForKind = useMemo<DetectedDbInstance[]>(() => {
    const all = instancesEntry?.instances ?? [];
    const detectedKind: DetectedKind =
      kind === "mysql" ? "mysql" : kind === "postgres" ? "postgres" : "redis";
    // Hide detections that were already adopted as saved credentials
    // (match by detection signature).
    const adopted = new Set(
      savedForKind
        .map((c) => (c.source.kind === "detected" ? c.source.signature : null))
        .filter((s): s is string => !!s),
    );
    return all.filter((d) => d.kind === detectedKind && !adopted.has(d.signature));
  }, [instancesEntry, kind, savedForKind]);

  // Lazy-trigger detection the first time the picker mounts for
  // a tab that has SSH context. Also re-triggers when stale.
  useEffect(() => {
    if (!sshTarget) return;
    const entry = instancesEntry;
    const fresh = entry?.status === "ready" && Date.now() - entry.at < DETECTION_TTL_MS;
    if (fresh || entry?.status === "pending") return;

    let cancelled = false;
    setPending(tab.id);
    cmd
      .dbDetect({
        host: sshTarget.host,
        port: sshTarget.port,
        user: sshTarget.user,
        authMode: sshTarget.authMode,
        password: sshTarget.password,
        keyPath: sshTarget.keyPath,
        savedConnectionIndex: sshTarget.savedConnectionIndex,
      })
      .then((report) => {
        if (cancelled) return;
        setInstances(tab.id, {
          instances: report.instances,
          mysqlCli: report.mysqlCli,
          psqlCli: report.psqlCli,
          redisCli: report.redisCli,
          sqliteCli: report.sqliteCli,
        });
      })
      .catch(() => {
        if (cancelled) return;
        setError(tab.id);
      });
    return () => {
      cancelled = true;
    };
    // We intentionally omit `instancesEntry` from deps — it's only
    // a guard to skip re-running while pending or still fresh, and
    // including it would trigger a re-fetch on every state update.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab.id, sshTarget?.host, sshTarget?.port, sshTarget?.user, kind]);

  async function refreshDetection() {
    if (!sshTarget) return;
    setPending(tab.id);
    try {
      const report = await cmd.dbDetect({
        host: sshTarget.host,
        port: sshTarget.port,
        user: sshTarget.user,
        authMode: sshTarget.authMode,
        password: sshTarget.password,
        keyPath: sshTarget.keyPath,
        savedConnectionIndex: sshTarget.savedConnectionIndex,
      });
      setInstances(tab.id, {
        instances: report.instances,
        mysqlCli: report.mysqlCli,
        psqlCli: report.psqlCli,
        redisCli: report.redisCli,
        sqliteCli: report.sqliteCli,
      });
    } catch {
      setError(tab.id);
    }
  }

  async function performDelete(cred: DbCredential) {
    if (savedIndex === null) return;
    setDeleteError("");
    try {
      await cmd.dbCredDelete(savedIndex, cred.id);
      await refreshConnections();
      onDeleted?.(cred);
      setDeleteTarget(null);
    } catch (e) {
      setDeleteError(formatError(e));
    }
  }

  const activeId = activeIdFor(tab, kind);
  const isPending = instancesEntry?.status === "pending";
  const hasSsh = sshTarget !== null;
  // Local tab: no auto-detection + no saved creds (saved creds
  // are attached to SSH profiles). Hide the picker entirely.
  if (!hasSsh && savedForKind.length === 0) {
    return null;
  }

  return (
    <section className="panel-section">
      <div className="panel-section__title">
        <span>{t("Instances")}</span>
        {hasSsh && (
          <span className="panel-section__hint">
            <button
              className="mini-button mini-button--ghost"
              disabled={isPending}
              onClick={() => void refreshDetection()}
              title={t("Refresh detection")}
              type="button"
            >
              <RefreshCw size={11} />
              {isPending ? t("Scanning...") : t("Refresh")}
            </button>
          </span>
        )}
      </div>
      <div className="db-instance-picker">
        {savedForKind.length > 0 && (
          <div className="db-instance-row">
            <span className="db-instance-row__label">{t("Saved")}</span>
            <div className="db-instance-row__pills">
              {savedForKind.map((cred) => {
                const selected = cred.id === activeId;
                return (
                  <span
                    key={cred.id}
                    className={
                      selected
                        ? "db-instance-pill db-instance-pill--saved db-instance-pill--selected"
                        : "db-instance-pill db-instance-pill--saved"
                    }
                  >
                    <button
                      className="db-instance-pill__body"
                      onClick={() => onActivate(cred)}
                      type="button"
                      title={`${cred.user}@${cred.host}:${cred.port}${cred.database ? ` · ${cred.database}` : ""}`}
                    >
                      {cred.favorite && (
                        <Star
                          size={10}
                          fill="currentColor"
                          style={{ color: "var(--warn)" }}
                        />
                      )}
                      <span className="db-instance-pill__name">{cred.label || cred.id}</span>
                      <span className="db-instance-pill__port">:{cred.port}</span>
                    </button>
                    {selected && (
                      <button
                        className="db-instance-pill__trail"
                        onClick={() => setDeleteTarget(cred)}
                        title={t("Delete")}
                        type="button"
                      >
                        <Trash2 size={10} />
                      </button>
                    )}
                  </span>
                );
              })}
            </div>
          </div>
        )}
        {hasSsh && (
          <div className="db-instance-row">
            <span className="db-instance-row__label">{t("Detected")}</span>
            <div className="db-instance-row__pills">
              {detectedForKind.length === 0 ? (
                <span className="db-instance-row__empty mono">
                  {isPending ? t("Scanning...") : t("None found")}
                </span>
              ) : (
                detectedForKind.map((det) => {
                  const SourceIcon = det.source === "docker" ? DockerIcon : Server;
                  return (
                    <button
                      key={det.signature}
                      className="db-instance-pill db-instance-pill--detected"
                      onClick={() => onAdopt(det)}
                      title={
                        det.source === "docker"
                          ? `${det.image ?? ""} · ${det.host}:${det.port}`
                          : `${det.processName ?? ""} · ${det.host}:${det.port}`
                      }
                      type="button"
                    >
                      <SourceIcon size={11} />
                      <span className="db-instance-pill__name">{det.label}</span>
                      <span className="db-instance-pill__port">:{det.port}</span>
                    </button>
                  );
                })
              )}
            </div>
          </div>
        )}
        <div className="db-instance-row db-instance-row--actions">
          <button
            className="mini-button"
            onClick={onAddNew}
            type="button"
            title={savedIndex === null ? t("Save to SSH profile unavailable on manual SSH") : undefined}
          >
            <Plus size={11} /> {t("Add connection")}
          </button>
        </div>
        {deleteError && (
          <div className="status-note status-note--error">{deleteError}</div>
        )}
      </div>
      <ConfirmDialog
        open={deleteTarget !== null}
        tone="destructive"
        title={t("Delete saved credential")}
        message={t("Delete saved credential {label}? This can't be undone.", {
          label: deleteTarget?.label || deleteTarget?.id || "",
        })}
        confirmLabel={t("Delete")}
        onCancel={() => {
          setDeleteTarget(null);
          setDeleteError("");
        }}
        onConfirm={() => {
          if (deleteTarget) void performDelete(deleteTarget);
        }}
      />
    </section>
  );
}

function activeIdFor(tab: TabState, kind: DetectedKind): string | null {
  switch (kind) {
    case "mysql":
      return tab.mysqlActiveCredentialId;
    case "postgres":
      return tab.pgActiveCredentialId;
    case "redis":
      return tab.redisActiveCredentialId;
  }
}
