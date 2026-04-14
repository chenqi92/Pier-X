import { X } from "lucide-react";
import { useState } from "react";
import type { RightTool, TabState } from "../lib/types";
import { useI18n } from "../i18n/useI18n";
import GitPanel from "../panels/GitPanel";
import MySqlPanel from "../panels/MySqlPanel";
import PostgresPanel from "../panels/PostgresPanel";
import SqlitePanel from "../panels/SqlitePanel";
import RedisPanel from "../panels/RedisPanel";
import DockerPanel from "../panels/DockerPanel";
import SftpPanel from "../panels/SftpPanel";
import ServerMonitorPanel from "../panels/ServerMonitorPanel";
import MarkdownPanel from "../panels/MarkdownPanel";
import LogViewerPanel from "../panels/LogViewerPanel";
import ToolStrip from "./ToolStrip";

type Props = {
  activeTab: TabState | null;
  browserPath: string;
  onToolChange: (tool: RightTool) => void;
};

const TOOL_TITLES: Record<string, string> = {
  git: "Git",
  monitor: "Server Monitor",
  docker: "Docker",
  mysql: "MySQL",
  postgres: "PostgreSQL",
  redis: "Redis",
  log: "Logs",
  sftp: "SFTP",
  sqlite: "SQLite",
  markdown: "Markdown",
};

function ToolContent({ tool, tab, browserPath }: { tool: RightTool; tab: TabState | null; browserPath: string }) {
  switch (tool) {
    case "git": return <GitPanel browserPath={browserPath} />;
    case "monitor": return tab ? <ServerMonitorPanel tab={tab} /> : <div className="empty-note">__OPEN_A_TAB_FIRST__</div>;
    case "docker": return tab ? <DockerPanel tab={tab} /> : <div className="empty-note">__OPEN_A_TAB_FIRST__</div>;
    case "mysql": return tab ? <MySqlPanel tab={tab} /> : <div className="empty-note">__OPEN_A_TAB_FIRST__</div>;
    case "postgres": return tab ? <PostgresPanel tab={tab} /> : <div className="empty-note">__OPEN_A_TAB_FIRST__</div>;
    case "redis": return tab ? <RedisPanel tab={tab} /> : <div className="empty-note">__OPEN_A_TAB_FIRST__</div>;
    case "log": return tab ? <LogViewerPanel tab={tab} /> : <div className="empty-note">__OPEN_A_TAB_FIRST__</div>;
    case "sftp": return tab ? <SftpPanel tab={tab} /> : <div className="empty-note">__OPEN_A_TAB_FIRST__</div>;
    case "sqlite": return <SqlitePanel />;
    case "markdown": return <MarkdownPanel />;
    default: return <div className="empty-note">__UNKNOWN_TOOL__</div>;
  }
}

export default function RightSidebar({ activeTab, browserPath, onToolChange }: Props) {
  const { t } = useI18n();
  const [expanded, setExpanded] = useState(true);

  const activeTool: RightTool = activeTab?.rightTool ?? "git";
  const hasRemoteContext = activeTab?.backend === "ssh";
  const title = TOOL_TITLES[activeTool] ?? activeTool;
  const isGitTool = activeTool === "git";
  const openTabFirst = t("Open a tab first.");
  const unknownTool = t("Unknown tool.");

  function renderToolContent() {
    const content = ToolContent({ tool: activeTool, tab: activeTab, browserPath });
    if (typeof content === "string") return content;
    return content;
  }

  const toolContent = ToolContent({ tool: activeTool, tab: activeTab, browserPath });
  const resolvedToolContent =
    typeof toolContent === "object" && toolContent !== null
      ? toolContent
      : toolContent;

  return (
    <div className="right-sidebar">
      {expanded && (
        isGitTool ? (
          <div className="right-sidebar__content right-sidebar__content--git">
            {toolContent}
          </div>
        ) : (
          <div className="right-sidebar__content">
            <div className="right-sidebar__header">
              <div>
                <h3 className="right-sidebar__title">{t(title)}</h3>
                {activeTab?.backend === "ssh" && (
                  <span className="right-sidebar__subtitle">
                    {activeTab.sshUser}@{activeTab.sshHost}:{activeTab.sshPort}
                  </span>
                )}
              </div>
              <button
                className="topbar__icon-btn"
                onClick={() => setExpanded(false)}
                title={t("Close")}
                type="button"
              >
                <X size={14} />
              </button>
            </div>
            <div className="right-sidebar__body">
              {toolContent}
            </div>
          </div>
        )
      )}
      <ToolStrip
        activeTool={activeTool}
        hasRemoteContext={hasRemoteContext}
        onSelectTool={(tool) => {
          onToolChange(tool);
          if (!expanded) setExpanded(true);
        }}
        expanded={expanded}
        onToggleExpand={() => setExpanded((p) => !p)}
      />
    </div>
  );
}
