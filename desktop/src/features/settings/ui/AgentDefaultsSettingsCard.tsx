import { AgentDefaultsEditor } from "@/features/agents/ui/AgentDefaultsEditor";
import { SettingsOptionGroup } from "./SettingsOptionGroup";

export function AgentDefaultsSettingsCard() {
  return (
    <SettingsOptionGroup
      data-testid="settings-global-agent-config"
      description="Provider, model, effort, and environment settings inherited by local agents. Agent-specific settings always take priority."
      title="Agent defaults"
    >
      <div className="px-4 py-4">
        <AgentDefaultsEditor layout="flat" />
      </div>
    </SettingsOptionGroup>
  );
}
