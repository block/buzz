export type AgentAiConfigurationMode = "defaults" | "custom";

export type AgentAiConfigurationPair = {
  provider: string;
  model: string;
};

export function initialAgentAiConfigurationMode(
  pair: Partial<AgentAiConfigurationPair>,
): AgentAiConfigurationMode {
  return pair.provider?.trim() || pair.model?.trim() ? "custom" : "defaults";
}

export function agentAiConfigurationPairForMode({
  current,
  inherited,
  mode,
}: {
  current: AgentAiConfigurationPair;
  inherited: AgentAiConfigurationPair;
  mode: AgentAiConfigurationMode;
}): AgentAiConfigurationPair {
  if (mode === "defaults") {
    return { provider: "", model: "" };
  }

  return {
    provider: current.provider.trim() || inherited.provider,
    model: current.model.trim() || inherited.model,
  };
}

export function agentAiConfigurationModeSatisfied(
  mode: AgentAiConfigurationMode,
  pair: AgentAiConfigurationPair,
) {
  return (
    mode === "defaults" ||
    (pair.provider.trim().length > 0 && pair.model.trim().length > 0)
  );
}
