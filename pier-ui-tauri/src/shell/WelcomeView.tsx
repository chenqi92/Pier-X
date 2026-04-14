import { PlugZap, SquareTerminal } from "lucide-react";
import { useI18n } from "../i18n/useI18n";
import { useConnectionStore } from "../stores/useConnectionStore";
import { useThemeStore } from "../stores/useThemeStore";

type Props = {
  onOpenLocalTerminal: () => void;
  onNewSsh: () => void;
  onConnectSaved: (index: number) => void;
  version?: string;
};

export default function WelcomeView({
  onOpenLocalTerminal,
  onNewSsh,
  onConnectSaved,
  version,
}: Props) {
  const { t } = useI18n();
  const { resolvedDark } = useThemeStore();
  const { connections } = useConnectionStore();
  const recent = connections.slice(0, 6);

  return (
    <div className="welcome">
      <div className="welcome__hero">
        <div className="welcome__icon">
          <span className="welcome__dot" />
        </div>
        <span className="welcome__tag">{t("Welcome")}</span>
        <h1 className="welcome__title">{t("Pier-X workspace")}</h1>
        <div className="welcome__actions">
          <button className="welcome__btn welcome__btn--primary" onClick={onNewSsh} type="button">
            <PlugZap size={15} />
            {t("New SSH connection")}
          </button>
          <button className="welcome__btn welcome__btn--ghost" onClick={onOpenLocalTerminal} type="button">
            <SquareTerminal size={15} />
            {t("Open local terminal")}
          </button>
        </div>
        <div className="welcome__pills">
          {version ? <span className="welcome__pill">v{version}</span> : null}
          <span className="welcome__pill">{resolvedDark ? "Dark" : "Light"}</span>
        </div>
      </div>

      {recent.length > 0 && (
        <div className="welcome__recent">
          <h3 className="welcome__section-title">{t("Recent connections")}</h3>
          <div className="welcome__grid">
            {recent.map((conn) => (
              <button
                key={conn.index}
                className="welcome__card"
                onClick={() => onConnectSaved(conn.index)}
                type="button"
              >
                <SquareTerminal size={16} />
                <div className="welcome__card-body">
                  <strong>{conn.name}</strong>
                  <span>{conn.user}@{conn.host}:{conn.port}</span>
                </div>
              </button>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
