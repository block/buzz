import { AgentDefaultsSettingsCard } from "./AgentDefaultsSettingsCard";
import {
  setKeepMentionedAgentsPinned,
  useKeepMentionedAgentsPinned,
} from "@/features/messages/lib/autoPinMentionedAgentsPreference";
import { useTranslation } from "@/i18n";
import { Switch } from "@/shared/ui/switch";
import { HarnessesSettingsPanel } from "./HarnessesSettingsPanel";
import { PreventSleepSettingsCard } from "./PreventSleepSettingsCard";
import {
  SettingsOptionGroup,
  SettingsOptionGroupList,
  SettingsOptionRow,
} from "./SettingsOptionGroup";
import { SettingsSectionHeader } from "./SettingsSectionHeader";

export function AgentsSettingsPanel() {
  const { t } = useTranslation();
  const automaticallyMentionAgents = useKeepMentionedAgentsPinned();

  return (
    <section className="min-w-0" data-testid="settings-agents">
      <SettingsSectionHeader
        title={t("settings.agents.title")}
        description={t("settings.agents.description")}
      />

      <SettingsOptionGroupList>
        <SettingsOptionGroup title={t("settings.agents.group-conversations")}>
          <SettingsOptionRow data-testid="settings-automatic-agent-mentions">
            <div className="min-w-0">
              <label
                className="font-medium text-foreground"
                htmlFor="settings-automatic-agent-mentions-switch"
              >
                {t("settings.agents.auto-mention")}
              </label>
              <p
                className="mt-0.5 text-sm text-muted-foreground/70"
                data-settings-subcopy
              >
                {t("settings.agents.auto-mention-hint")}
              </p>
            </div>
            <Switch
              aria-label={t("settings.agents.auto-mention")}
              checked={automaticallyMentionAgents}
              id="settings-automatic-agent-mentions-switch"
              onCheckedChange={setKeepMentionedAgentsPinned}
            />
          </SettingsOptionRow>
        </SettingsOptionGroup>
        <PreventSleepSettingsCard />
        <HarnessesSettingsPanel />
        <AgentDefaultsSettingsCard />
      </SettingsOptionGroupList>
    </section>
  );
}
