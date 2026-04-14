import { Plus, X } from "lucide-react";
import { useI18n } from "../i18n/useI18n";
import { TAB_COLORS } from "../lib/types";
import { useTabStore } from "../stores/useTabStore";

type Props = {
  onNewTab: () => void;
};

export default function TabBar({ onNewTab }: Props) {
  const { t } = useI18n();
  const { tabs, activeTabId, setActiveTab, closeTab } = useTabStore();

  if (tabs.length === 0) return null;

  return (
    <div className="tabbar">
      <div className="tabbar__scroll">
        {tabs.map((tab) => {
          const isActive = tab.id === activeTabId;
          const colorDot =
            tab.tabColor >= 0 && tab.tabColor < TAB_COLORS.length
              ? TAB_COLORS[tab.tabColor].value
              : null;

          return (
            <button
              key={tab.id}
              className={isActive ? "tab tab--active" : "tab"}
              onClick={() => setActiveTab(tab.id)}
              type="button"
            >
              {colorDot ? (
                <span
                  className="tab__color"
                  style={{ backgroundColor: colorDot }}
                />
              ) : null}
              <span className="tab__title">{tab.title}</span>
              <span
                className="tab__close"
                onClick={(e) => {
                  e.stopPropagation();
                  closeTab(tab.id);
                }}
                role="button"
                tabIndex={-1}
              >
                <X size={12} />
              </span>
            </button>
          );
        })}
      </div>
      <button
        className="tabbar__add"
        onClick={onNewTab}
        title={t("New tab")}
        type="button"
      >
        <Plus size={14} />
      </button>
    </div>
  );
}
