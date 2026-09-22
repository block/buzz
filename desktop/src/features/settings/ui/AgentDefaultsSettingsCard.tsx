import { AgentDefaultsEditor } from "@/features/agents/ui/AgentDefaultsEditor";
import { useTranslation } from "@/i18n";
import { SettingsOptionGroup } from "./SettingsOptionGroup";

export function AgentDefaultsSettingsCard() {
  const { t } = useTranslation();

  return (
    <SettingsOptionGroup
      data-testid="settings-global-agent-config"
      description={t("settings.agent-defaults.description")}
      title={t("settings.agent-defaults.title")}
    >
      <div className="px-4 py-4">
        <AgentDefaultsEditor layout="flat" />
      </div>
    </SettingsOptionGroup>
  );
}
