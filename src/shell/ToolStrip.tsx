import {
  ChevronsLeft,
  ChevronsRight,
} from "lucide-react";
import type { RightTool } from "../lib/types";
import { RIGHT_TOOL_META, RIGHT_TOOL_ORDER } from "../lib/rightToolMeta";
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

const TOOLS = RIGHT_TOOL_ORDER.map((tool) => ({ tool, ...RIGHT_TOOL_META[tool] }));

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
