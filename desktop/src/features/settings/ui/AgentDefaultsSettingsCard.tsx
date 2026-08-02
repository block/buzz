import { AgentDefaultsEditor } from "@/features/agents/ui/AgentDefaultsEditor";
import { SectionHeader } from "@/shared/ui/PageHeader";
import { useTranslation } from "react-i18next";

export function AgentDefaultsSettingsCard() {
  const { t } = useTranslation();
  return (
    <section
      className="min-w-0 space-y-4"
      data-testid="settings-global-agent-config"
    >
      <SectionHeader
        title={t("agents.defaults")}
        description={t("agents.defaultsDescription")}
      />
      <AgentDefaultsEditor />
    </section>
  );
}
