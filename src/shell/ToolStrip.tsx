import {
  ActivitySquare,
  ChevronsLeft,
  ChevronsRight,
  Container,
  Database,
  FileText,
  FolderTree,
  GitBranch,
  HardDrive,
  ScrollText,
  Zap,
} from "lucide-react";
import type { RightTool } from "../lib/types";
import { useI18n } from "../i18n/useI18n";
import ToolStripItem from "../components/ToolStripItem";

type Props = {
  activeTool: RightTool;
  onSelectTool: (tool: RightTool) => void;
  hasRemoteContext: boolean;
  detectedTools?: ReadonlySet<RightTool>;
  expanded: boolean;
  onToggleExpand: () => void;
};

const TOOLS: {
  tool: RightTool;
  icon: typeof GitBranch;
  label: string;
  remoteOnly?: boolean;
  dividerAfter?: boolean;
}[] = [
  { tool: "markdown", icon: FileText, label: "Markdown" },
  { tool: "git", icon: GitBranch, label: "Git", dividerAfter: true },
  { tool: "monitor", icon: ActivitySquare, label: "Server Monitor" },
  { tool: "docker", icon: Container, label: "Docker" },
  { tool: "mysql", icon: Database, label: "MySQL" },
  { tool: "postgres", icon: Database, label: "PostgreSQL" },
  { tool: "redis", icon: Zap, label: "Redis" },
  { tool: "log", icon: ScrollText, label: "Logs" },
  { tool: "sftp", icon: FolderTree, label: "SFTP", remoteOnly: true },
  { tool: "sqlite", icon: HardDrive, label: "SQLite" },
];

export default function ToolStrip({ activeTool, onSelectTool, hasRemoteContext, detectedTools, expanded, onToggleExpand }: Props) {
  const { t } = useI18n();

  return (
    <div className="toolstrip">
      {TOOLS.map((entry) => {
        const isActive = activeTool === entry.tool;
        const dim = entry.remoteOnly && !hasRemoteContext;
        const detected = detectedTools?.has(entry.tool) ?? false;
        return (
          <div key={entry.tool} style={{ display: "contents" }}>
            <ToolStripItem
              icon={entry.icon}
              label={t(entry.label)}
              active={isActive}
              dim={dim}
              detected={detected}
              onClick={() => {
                if (dim) return;
                onSelectTool(entry.tool);
              }}
            />
            {entry.dividerAfter && <div className="ts-divider" />}
          </div>
        );
      })}
      <div className="toolstrip-spacer" />
      <button
        type="button"
        className="ts-btn"
        onClick={onToggleExpand}
        title={expanded ? t("Collapse") : t("Expand")}
      >
        {expanded ? <ChevronsRight size={16} /> : <ChevronsLeft size={16} />}
      </button>
    </div>
  );
}
