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

/**
 * Whether a Customize (explicit) AI pair is complete enough to submit.
 *
 * `needsProviderSelection` mirrors `runtimeSupportsLlmProviderSelection`:
 * Buzz Agent / Goose expose a provider picker and require both provider and
 * model, while Codex / Claude drive their own provider and hide the field, so
 * requiring a provider there would gate Save on a value the user can never set
 * (the create/edit "Save stays disabled" regression). It defaults to `true` so
 * existing callers keep the provider+model requirement.
 */
export function agentAiConfigurationModeSatisfied(
  mode: AgentAiConfigurationMode,
  pair: AgentAiConfigurationPair,
  needsProviderSelection = true,
) {
  if (mode === "defaults") {
    return true;
  }
  const providerOk = !needsProviderSelection || pair.provider.trim().length > 0;
  return providerOk && pair.model.trim().length > 0;
}
