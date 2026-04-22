import type { RightTool } from "../lib/types";
import { RIGHT_TOOL_META, RIGHT_TOOL_ORDER } from "../lib/rightToolMeta";
import { useI18n } from "../i18n/useI18n";
import ToolStripItem from "../components/ToolStripItem";

type Props = {
  activeTool: RightTool;
  onSelectTool: (tool: RightTool) => void;
  hasRemoteContext: boolean;
  detectedTools?: ReadonlySet<RightTool>;
};

const TOOLS = RIGHT_TOOL_ORDER.map((tool) => ({ tool, ...RIGHT_TOOL_META[tool] }));

export default function ToolStrip({ activeTool, onSelectTool, hasRemoteContext, detectedTools }: Props) {
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
    </div>
  );
}
