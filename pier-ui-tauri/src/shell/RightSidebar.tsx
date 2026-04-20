import type { RightTool, TabState } from "../lib/types";
import { useState } from "react";
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
  selectedMarkdownPath: string;
  onToolChange: (tool: RightTool) => void;
};

function ToolContent({
  tool,
  tab,
  browserPath,
  markdownPath,
  openTabFirstLabel,
  unknownToolLabel,
}: {
  tool: RightTool;
  tab: TabState | null;
  browserPath: string;
  markdownPath: string;
  openTabFirstLabel: string;
  unknownToolLabel: string;
}) {
  switch (tool) {
    case "git": return <GitPanel browserPath={browserPath} />;
    case "monitor": return tab ? <ServerMonitorPanel tab={tab} /> : <div className="empty-note">{openTabFirstLabel}</div>;
    case "docker": return tab ? <DockerPanel tab={tab} /> : <div className="empty-note">{openTabFirstLabel}</div>;
    case "mysql": return tab ? <MySqlPanel tab={tab} /> : <div className="empty-note">{openTabFirstLabel}</div>;
    case "postgres": return tab ? <PostgresPanel tab={tab} /> : <div className="empty-note">{openTabFirstLabel}</div>;
    case "redis": return tab ? <RedisPanel tab={tab} /> : <div className="empty-note">{openTabFirstLabel}</div>;
    case "log": return tab ? <LogViewerPanel tab={tab} /> : <div className="empty-note">{openTabFirstLabel}</div>;
    case "sftp": return tab ? <SftpPanel tab={tab} /> : <div className="empty-note">{openTabFirstLabel}</div>;
    case "sqlite": return <SqlitePanel />;
    case "markdown": return <MarkdownPanel filePath={markdownPath} />;
    default: return <div className="empty-note">{unknownToolLabel}</div>;
  }
}

/**
 * RightSidebar owns the right grid cell — a rightpanel (panel content) and
 * the ToolStrip rail beside it. Each panel renders its own PanelHeader
 * internally, so RightSidebar only provides the frame.
 */
export default function RightSidebar({ activeTab, browserPath, selectedMarkdownPath, onToolChange }: Props) {
  const { t } = useI18n();
  const [expanded, setExpanded] = useState(true);

  const activeTool: RightTool = activeTab?.rightTool ?? "markdown";
  const hasRemoteContext = activeTab?.backend === "ssh";
  const openTabFirst = t("Open a tab first.");
  const unknownTool = t("Unknown tool.");

  return (
    <div className="right-sidebar">
      {expanded && (
        <div className="right-sidebar__content">
          <ToolContent
            tool={activeTool}
            tab={activeTab}
            browserPath={browserPath}
            markdownPath={selectedMarkdownPath}
            openTabFirstLabel={openTabFirst}
            unknownToolLabel={unknownTool}
          />
        </div>
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
