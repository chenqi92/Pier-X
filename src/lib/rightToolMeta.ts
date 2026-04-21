import {
  ChartNoAxesCombined,
  Container,
  Database,
  FileText,
  FolderSync,
  GitBranch,
  KeyRound,
  Logs,
  Table2,
  TableProperties,
} from "lucide-react";
import type { ComponentType, SVGProps } from "react";
import type { RightTool } from "./types";

export type LucideIcon = ComponentType<SVGProps<SVGSVGElement> & { size?: number | string }>;

export type RightToolMeta = {
  label: string;
  icon: LucideIcon;
  remoteOnly?: boolean;
  dividerAfter?: boolean;
  tintVar?: string;
  splashTitle?: string;
  splashSubtitle?: string;
};

export const RIGHT_TOOL_ORDER: RightTool[] = [
  "markdown",
  "git",
  "monitor",
  "docker",
  "mysql",
  "postgres",
  "redis",
  "log",
  "sftp",
  "sqlite",
];

export const SERVICE_CHIP_TOOLS: RightTool[] = [
  "docker",
  "mysql",
  "postgres",
  "redis",
  "monitor",
  "log",
  "sftp",
  "sqlite",
];

export const RIGHT_TOOL_META: Record<RightTool, RightToolMeta> = {
  markdown: {
    label: "Markdown",
    icon: FileText,
  },
  git: {
    label: "Git",
    icon: GitBranch,
    dividerAfter: true,
  },
  monitor: {
    label: "Server Monitor",
    icon: ChartNoAxesCombined,
    remoteOnly: true,
    tintVar: "var(--svc-monitor)",
    splashTitle: "Server Monitor",
    splashSubtitle: "Open a saved server to see live CPU, memory, disks, and top processes.",
  },
  docker: {
    label: "Docker",
    icon: Container,
    remoteOnly: true,
    tintVar: "var(--svc-docker)",
    splashTitle: "Docker",
    splashSubtitle: "Pick a host to list containers, images, networks, and compose stacks.",
  },
  mysql: {
    label: "MySQL",
    icon: TableProperties,
    remoteOnly: true,
    tintVar: "var(--svc-mysql)",
    splashTitle: "MySQL",
    splashSubtitle: "Connect through SSH to browse databases, run queries, and edit rows.",
  },
  postgres: {
    label: "PostgreSQL",
    icon: Table2,
    remoteOnly: true,
    tintVar: "var(--svc-postgres)",
    splashTitle: "PostgreSQL",
    splashSubtitle: "Connect through SSH to explore schemas, tables, and run SQL.",
  },
  redis: {
    label: "Redis",
    icon: KeyRound,
    remoteOnly: true,
    tintVar: "var(--svc-redis)",
    splashTitle: "Redis",
    splashSubtitle: "Tunnel into a host to browse keyspaces, inspect values, and tail keys.",
  },
  log: {
    label: "Logs",
    icon: Logs,
    remoteOnly: true,
    tintVar: "var(--svc-log)",
    splashTitle: "Log Viewer",
    splashSubtitle: "Stream journal, nginx, or custom log tails from a saved server.",
  },
  sftp: {
    label: "SFTP",
    icon: FolderSync,
    remoteOnly: true,
    tintVar: "var(--svc-sftp)",
    splashTitle: "SFTP",
    splashSubtitle: "Browse a remote filesystem, preview files, and transfer in either direction.",
  },
  sqlite: {
    label: "SQLite",
    icon: Database,
    tintVar: "var(--svc-sqlite)",
  },
};
