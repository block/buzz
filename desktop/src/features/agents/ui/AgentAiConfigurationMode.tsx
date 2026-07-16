import { Tabs, TabsList, TabsTrigger } from "@/shared/ui/tabs";
import type { AgentAiConfigurationMode } from "./agentAiConfigurationPolicy";

export type { AgentAiConfigurationMode } from "./agentAiConfigurationPolicy";

export function AgentAiConfigurationModeField({
  mode,
  onModeChange,
}: {
  mode: AgentAiConfigurationMode;
  onModeChange: (mode: AgentAiConfigurationMode) => void;
}) {
  return (
    <div className="space-y-1.5">
      <p className="text-sm font-medium text-foreground">AI configuration</p>
      <Tabs
        onValueChange={(value) =>
          onModeChange(value as AgentAiConfigurationMode)
        }
        value={mode}
      >
        <TabsList>
          <TabsTrigger value="defaults">Use AI defaults</TabsTrigger>
          <TabsTrigger value="custom">Customize for this agent</TabsTrigger>
        </TabsList>
      </Tabs>
      {mode === "custom" ? (
        <p className="text-xs text-muted-foreground">
          Provider and model changes apply only to this agent.
        </p>
      ) : null}
    </div>
  );
}
