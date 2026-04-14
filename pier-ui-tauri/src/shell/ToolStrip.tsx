import {
  ActivitySquare,
  Container,
  Database,
  FolderTree,
  GitBranch,
  HardDrive,
  ScrollText,
  Zap,
} from "lucide-react";
import type { RightTool } from "../lib/types";
import { useI18n } from "../i18n/useI18n";

type Props = {
  activeTool: RightTool;
  onSelectTool: (tool: RightTool) => void;
  hasRemoteContext: boolean;
};

const TOOLS: { tool: RightTool; icon: typeof GitBranch; label: string; remoteOnly?: boolean }[] = [
  { tool: "git", icon: GitBranch, label: "Git" },
  { tool: "monitor", icon: ActivitySquare, label: "Server Monitor" },
  { tool: "docker", icon: Container, label: "Docker" },
  { tool: "mysql", icon: Database, label: "MySQL" },
  { tool: "postgres", icon: Database, label: "PostgreSQL" },
  { tool: "redis", icon: Zap, label: "Redis" },
  { tool: "log", icon: ScrollText, label: "Logs" },
  { tool: "sftp", icon: FolderTree, label: "SFTP", remoteOnly: true },
  { tool: "sqlite", icon: HardDrive, label: "SQLite" },
];

export default function ToolStrip({ activeTool, onSelectTool, hasRemoteContext }: Props) {
  const { t } = useI18n();

  return (
    <div className="tool-strip">
      {TOOLS.map((entry, i) => {
        const Icon = entry.icon;
        const isActive = activeTool === entry.tool;
        const available = !entry.remoteOnly || hasRemoteContext;

        return (
          <div key={entry.tool}>
            {i === 1 && <div className="tool-strip__divider" />}
            <button
              className={
                isActive
                  ? "tool-strip__btn tool-strip__btn--active"
                  : "tool-strip__btn"
              }
              disabled={!available}
              onClick={() => onSelectTool(entry.tool)}
              title={t(entry.label)}
              type="button"
            >
              {isActive && <span className="tool-strip__indicator" />}
              <Icon size={16} />
            </button>
          </div>
        );
      })}
    </div>
  );
}
