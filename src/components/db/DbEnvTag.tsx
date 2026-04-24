import { useI18n } from "../../i18n/useI18n";
import type { DbEnv } from "./dbTheme";

type Props = { env: DbEnv };

/**
 * Small tag rendered alongside a DB instance name to tell `prod`,
 * `stage`, `dev` and `local` apart at a glance. Purely visual — the
 * `env` input is inferred from labels until the backend stores an
 * explicit tag on the credential.
 */
export default function DbEnvTag({ env }: Props) {
  const { t } = useI18n();
  if (env === "unknown") return null;
  const label =
    env === "prod"
      ? t("prod")
      : env === "stage"
        ? t("stage")
        : env === "dev"
          ? t("dev")
          : t("local");
  return (
    <span className="db-env-tag" data-env={env}>
      {label}
    </span>
  );
}
