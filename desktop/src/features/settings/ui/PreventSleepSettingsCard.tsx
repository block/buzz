import { usePreventSleepContext } from "@/features/agents/usePreventSleep";
import { useTranslation } from "@/i18n";
import { Switch } from "@/shared/ui/switch";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";

export function PreventSleepSettingsCard() {
  const { t } = useTranslation();
  const { enabled, setEnabled, hasRunningAgents, expired, clearExpired } =
    usePreventSleepContext();

  return (
    <div className="min-w-0 space-y-3">
      <SettingsOptionGroup
        data-testid="agents-preferences-card"
        title={t("settings.common.group-preferences")}
      >
        <SettingsOptionRow>
          <div className="min-w-0">
            <label
              className="text-sm font-medium"
              htmlFor="prevent-sleep-switch"
            >
              {t("settings.prevent-sleep.label")}
            </label>
            <p
              className="text-sm font-normal text-muted-foreground/70"
              data-settings-subcopy
            >
              {t("settings.prevent-sleep.hint")}
            </p>
          </div>
          <Switch
            checked={enabled}
            data-testid="prevent-sleep-toggle"
            id="prevent-sleep-switch"
            onCheckedChange={(checked) => {
              if (expired) {
                clearExpired();
              }
              setEnabled(checked);
            }}
          />
        </SettingsOptionRow>
      </SettingsOptionGroup>

      {enabled && !hasRunningAgents && (
        <p className="mt-3 text-sm text-muted-foreground">
          {t("settings.prevent-sleep.waiting")}
        </p>
      )}

      {expired && (
        <p className="mt-3 rounded-xl border border-yellow-500/30 bg-yellow-500/10 px-3 py-2 text-sm text-yellow-700 dark:text-yellow-400">
          {t("settings.prevent-sleep.expired")}
        </p>
      )}
    </div>
  );
}
