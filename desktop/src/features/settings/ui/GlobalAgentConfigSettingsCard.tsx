import { GlobalAgentConfigEditor } from "@/features/agents/ui/GlobalAgentConfigEditor";
import { SectionHeader } from "@/shared/ui/PageHeader";

export function GlobalAgentConfigSettingsCard() {
  return (
    <section
      className="min-w-0 space-y-4"
      data-testid="settings-global-agent-config"
    >
      <SectionHeader
        title="AI defaults"
        description="Provider, model, effort, and environment settings inherited by local agents. Agent-specific settings always take priority."
      />
      <GlobalAgentConfigEditor />
    </section>
  );
}
