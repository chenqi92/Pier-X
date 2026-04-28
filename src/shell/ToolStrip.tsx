import { PanelRightClose, PanelRightOpen } from "lucide-react";
import type { RightTool } from "../lib/types";
import {
  CATEGORY_LABELS,
  RIGHT_TOOL_META,
  RIGHT_TOOL_ORDER,
} from "../lib/rightToolMeta";
import { useI18n } from "../i18n/useI18n";
import ToolStripItem from "../components/ToolStripItem";

type Props = {
  activeTool: RightTool;
  onSelectTool: (tool: RightTool) => void;
  hasRemoteContext: boolean;
  detectedTools?: ReadonlySet<RightTool>;
  collapsed: boolean;
  onToggleCollapsed: () => void;
};

const TOOLS = RIGHT_TOOL_ORDER.map((tool) => ({ tool, ...RIGHT_TOOL_META[tool] }));

export default function ToolStrip({
  activeTool,
  onSelectTool,
  hasRemoteContext,
  detectedTools,
  collapsed,
  onToggleCollapsed,
}: Props) {
  const { t } = useI18n();
  const collapseTitle = collapsed ? t("Show right panel") : t("Hide right panel");

  // Insert a thin divider between adjacent items whose `category`
  // changes. This visually clusters related tools (workspace ·
  // host · files · containers · database · service) without needing
  // text labels in the narrow strip.
  return (
    <div className="toolstrip">
      {TOOLS.map((entry, i) => {
        const isActive = activeTool === entry.tool;
        const dim = entry.remoteOnly && !hasRemoteContext;
        const detected = detectedTools?.has(entry.tool) ?? false;
        const prevCategory = i > 0 ? TOOLS[i - 1].category : null;
        const showDivider =
          prevCategory !== null && prevCategory !== entry.category;
        return (
          <div key={entry.tool} style={{ display: "contents" }}>
            {showDivider && (
              <div
                className="ts-divider"
                title={t(CATEGORY_LABELS[entry.category])}
              />
            )}
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
          </div>
        );
      })}
      <div className="toolstrip-spacer" />
      <div className="ts-divider" />
      <button
        type="button"
        className="ts-btn"
        title={collapseTitle}
        aria-label={collapseTitle}
        onClick={onToggleCollapsed}
      >
        {collapsed ? <PanelRightOpen size={16} /> : <PanelRightClose size={16} />}
      </button>
    </div>
  );
}
