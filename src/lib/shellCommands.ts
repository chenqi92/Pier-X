import { invoke } from "@tauri-apps/api/core";
import type {
  CoreInfo,
  DbDetectionReport,
  DetectedServiceView,
  EgressProfile,
  FileEntry,
  SavedSshConnection,
} from "./types";

export type SshParams = {
  host: string;
  port: number;
  user: string;
  authMode: string;
  password: string;
  keyPath: string;
  savedConnectionIndex?: number | null;
};

export type HostHealthReport = {
  savedConnectionIndex: number;
  status: "online" | "offline" | "timeout" | "error";
  latencyMs: number | null;
  errorMessage: string;
  checkedAt: number;
};

export type HostKeyPromptKind = "unknown" | "changed";

export type HostKeyPromptRequest = {
  host: string;
  port: number;
  keyType: string;
  fingerprint: string;
  kind: HostKeyPromptKind;
};

export type HostKeyPromptEvent = {
  id: string;
  request: HostKeyPromptRequest;
};

export const coreInfo = () => invoke<CoreInfo>("core_info");
export const devToggleDevtools = () => invoke<void>("dev_toggle_devtools");

export const listDirectory = (path?: string) =>
  invoke<FileEntry[]>("list_directory", { path: path ?? null });
export const listDrives = () => invoke<FileEntry[]>("list_drives");
export const localCreateFile = (path: string) =>
  invoke<void>("local_create_file", { path });
export const localCreateDir = (path: string) =>
  invoke<void>("local_create_dir", { path });
export const localRename = (from: string, to: string) =>
  invoke<void>("local_rename", { from, to });
export const localRemove = (path: string, isDir: boolean) =>
  invoke<void>("local_remove", { path, isDir });
export const localReadTextFile = (path: string) =>
  invoke<string>("local_read_text_file", { path });
export const localWriteTextFile = (path: string, content: string) =>
  invoke<void>("local_write_text_file", { path, content });

export const gitPush = (path: string | null) =>
  invoke<string>("git_push", { path });
export const gitPull = (path: string | null) =>
  invoke<string>("git_pull", { path });
export const gitFetchRemote = (path: string | null, name?: string | null) =>
  invoke<string>("git_fetch_remote", { path, name: name ?? null });

export const sshConnectionsList = () =>
  invoke<SavedSshConnection[]>("ssh_connections_list");
export const sshConnectionSave = (params: {
  name: string;
  host: string;
  port: number;
  user: string;
  authKind: string;
  password: string;
  keyPath: string;
  group?: string | null;
  envTag?: string | null;
  egressId?: string | null;
  autoElevate?: boolean;
  protocol?: "ssh" | "rdp" | "vnc";
  domain?: string | null;
}) => invoke<void>("ssh_connection_save", {
  name: params.name,
  host: params.host,
  port: params.port,
  user: params.user,
  authMode: params.authKind,
  password: params.password || null,
  keyPath: params.keyPath || null,
  group: params.group && params.group.trim() ? params.group.trim() : null,
  envTag: params.envTag && params.envTag.trim() ? params.envTag.trim() : null,
  egressId: params.egressId && params.egressId.trim() ? params.egressId.trim() : null,
  autoElevate: params.autoElevate ?? false,
  protocol: params.protocol ?? "ssh",
  domain: params.domain && params.domain.trim() ? params.domain.trim() : null,
});
export const sshConnectionUpdate = (params: {
  index: number;
  name: string;
  host: string;
  port: number;
  user: string;
  authKind: string;
  password: string;
  keyPath: string;
  group?: string | null;
  envTag?: string | null;
  egressId?: string | null;
  autoElevate?: boolean;
  protocol?: "ssh" | "rdp" | "vnc";
  domain?: string | null;
}) => invoke<void>("ssh_connection_update", {
  index: params.index,
  name: params.name,
  host: params.host,
  port: params.port,
  user: params.user,
  authMode: params.authKind,
  password: params.password || null,
  keyPath: params.keyPath || null,
  protocol: params.protocol ?? "ssh",
  domain: params.domain && params.domain.trim() ? params.domain.trim() : null,
  group: params.group === undefined
    ? null
    : params.group && params.group.trim() ? params.group.trim() : "",
  envTag:
    params.envTag === undefined
      ? null
      : params.envTag && params.envTag.trim()
        ? params.envTag.trim()
        : "",
  egressId:
    params.egressId === undefined
      ? null
      : params.egressId && params.egressId.trim()
        ? params.egressId.trim()
        : "",
  autoElevate: params.autoElevate,
});
export const sshConnectionDelete = (index: number) =>
  invoke<void>("ssh_connection_delete", { index });
export const sshConnectionsReorder = (
  order: number[],
  groups: Array<string | null>,
) => invoke<void>("ssh_connections_reorder", { order, groups });
export const sshGroupRename = (from: string, to: string | null) =>
  invoke<void>("ssh_group_rename", { from, to });
export const sshConnectionResolvePassword = (index: number) =>
  invoke<string | null>("ssh_connection_resolve_password", { index });
export const sshTunnelClose = (tunnelId: string) =>
  invoke<void>("ssh_tunnel_close", { tunnelId });
export const sshHostKeyDecide = (promptId: string, accept: boolean) =>
  invoke<void>("ssh_host_key_decide", { promptId, accept });

/** Garbage-collect cached panel SSH sessions. `active` is the set of
 *  `user@host:port` targets still referenced by open tabs; the backend
 *  tears down any cached session not in the set. Called after a tab
 *  closes so connections for hosts with no open tab don't leak. */
export const sshSessionsRetain = (active: string[]) =>
  invoke<void>("ssh_sessions_retain", { active });

export const hostHealthProbe = (params: {
  indices: number[];
  timeoutMs: number;
}) =>
  invoke<HostHealthReport[]>("host_health_probe", {
    indices: params.indices,
    timeoutMs: params.timeoutMs,
  });

export const egressProfileList = () =>
  invoke<EgressProfile[]>("egress_profile_list");
export const egressProfileSave = (profile: EgressProfile) =>
  invoke<void>("egress_profile_save", { profile });
export const egressProfileDelete = (id: string) =>
  invoke<void>("egress_profile_delete", { id });
export const egressSetBasicAuth = (
  credentialId: string,
  user: string,
  password: string,
) => invoke<void>("egress_set_basic_auth", { credentialId, user, password });
export const egressClearCredential = (credentialId: string) =>
  invoke<void>("egress_clear_credential", { credentialId });
export const egressVpnStart = (id: string) =>
  invoke<void>("egress_vpn_start", { id });
export const egressVpnStop = (id: string) =>
  invoke<void>("egress_vpn_stop", { id });
export const egressVpnStatusAll = () =>
  invoke<Record<string, boolean>>("egress_vpn_status_all");

export const setElevationPassword = (
  user: string,
  host: string,
  port: number,
  password: string,
) => invoke<void>("set_elevation_password", { user, host, port, password });
export const getElevationPassword = (
  user: string,
  host: string,
  port: number,
) => invoke<string | null>("get_elevation_password", { user, host, port });
export const forgetElevationPassword = (
  user: string,
  host: string,
  port: number,
) => invoke<void>("forget_elevation_password", { user, host, port });

export const detectServices = (params: SshParams) =>
  invoke<DetectedServiceView[]>("detect_services", params);
export const dbDetect = (params: SshParams) =>
  invoke<DbDetectionReport>("db_detect", params);

export const sftpDownload = (params: SshParams & {
  remotePath: string;
  localPath: string;
  transferId?: string | null;
}) => invoke<void>("sftp_download", params);
export const sftpDownloadTree = (params: SshParams & {
  remotePath: string;
  localPath: string;
  transferId?: string | null;
  concurrency?: number | null;
}) => invoke<void>("sftp_download_tree", params);
