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
  needsProviderSelection = true,
}: {
  current: AgentAiConfigurationPair;
  inherited: AgentAiConfigurationPair;
  mode: AgentAiConfigurationMode;
  needsProviderSelection?: boolean;
}): AgentAiConfigurationPair {
  if (mode === "defaults") {
    return { provider: "", model: "" };
  }

  return {
    provider: needsProviderSelection
      ? current.provider.trim() || inherited.provider
      : "",
    model: current.model.trim() || inherited.model,
  };
}

/**
 * Whether a Customize (explicit) AI pair is complete enough to submit.
 *
 * `needsProviderSelection` reflects whether the provider picker is actually
 * shown to the user: Buzz Agent / Goose expose it (and runtime-less legacy /
 * builtin definitions do too), so both provider and model are required, while
 * Codex / Claude drive their own provider and hide the field, so requiring a
 * provider there would gate Save on a value the user can never set (the
 * create/edit "Save stays disabled" regression). Callers should pass the
 * field-visibility capability (`runtimeCanChooseLlmProvider`), not the raw
 * runtime capability, so the gate never diverges from the visible picker. It
 * defaults to `true` so existing callers keep the provider+model requirement.
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
