import { Tabs, TabsList, TabsTrigger } from "@/shared/ui/tabs";
import type { AgentAiConfigurationMode } from "./agentAiConfigurationPolicy";

export type { AgentAiConfigurationMode } from "./agentAiConfigurationPolicy";

export function HarnessModelDefaultNotice({
  model,
}: {
  model?: string | null;
}) {
  return (
    <div className="text-sm" data-testid="agent-harness-defaults-notice">
      <span className="text-muted-foreground">Model</span>{" "}
      <span className="text-foreground">
        {model?.trim() || "Harness default"}
      </span>
    </div>
  );
}

export function AgentAiConfigurationModeField({
  mode,
  needsProviderSelection = true,
  onModeChange,
}: {
  mode: AgentAiConfigurationMode;
  needsProviderSelection?: boolean;
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
          <TabsTrigger value="defaults">
            {needsProviderSelection
              ? "Use agent defaults"
              : "Use harness defaults"}
          </TabsTrigger>
          <TabsTrigger value="custom">Customize for this agent</TabsTrigger>
        </TabsList>
      </Tabs>
      {mode === "custom" ? (
        <p className="text-xs text-muted-foreground">
          {needsProviderSelection
            ? "Provider and model changes apply only to this agent."
            : "Model changes apply only to this agent."}
        </p>
      ) : null}
    </div>
  );
}
