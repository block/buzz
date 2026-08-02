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

/** How many catalog ids the miss message names before it summarizes the rest. */
const MAX_LISTED_MODELS = 12;

/**
 * Why a typed model cannot be submitted, or `null` when it can.
 *
 * The Custom model input persists free text verbatim, and the runtime matches
 * a model against the harness's catalog byte-exactly — so a near miss
 * ("claude-fable-5" against a catalog offering "fable") does not fail loudly.
 * It misses, the adapter falls back to its own default, and the agent silently
 * runs a model the user did not choose. The submit gate is the last place that
 * still knows both strings, so the mismatch is caught here or not at all.
 *
 * Only checked when the harness actually published a catalog: a BYOH harness
 * may publish none, and free text is then the only way to name a model at all.
 *
 * `isTypedEntry` must mean "the user is typing into a live Custom model input
 * right now", not "the current model is outside the catalog". A saved model
 * absent from the catalog is the state this gate exists to prevent, but it is
 * also the state a user is already in when they open the dialog to edit
 * something else — gating on it would brick Save on an untouched field and
 * force them to re-pick a model to rename an agent. Catch it on the way in,
 * and leave the healing of already-saved values to the catalog-mismatch paths
 * that own it.
 */
export function typedModelCatalogError({
  catalog,
  isTypedEntry,
  model,
}: {
  catalog: readonly { id: string }[] | null;
  isTypedEntry: boolean;
  model: string;
}): string | null {
  const typed = model.trim();
  if (!isTypedEntry || typed.length === 0 || catalog === null) return null;
  // The synthetic "Default model" row carries an empty id and names no model.
  const ids = catalog.map((option) => option.id.trim()).filter(Boolean);
  if (ids.length === 0 || ids.includes(typed)) return null;
  const rest = ids.length - MAX_LISTED_MODELS;
  return `This harness has no model "${typed}". It offers: ${ids
    .slice(0, MAX_LISTED_MODELS)
    .join(", ")}${rest > 0 ? `, and ${rest} more` : ""}.`;
}

/**
 * The Model control's gate and status line in one pass.
 *
 * A catalog miss blocks Save, so it outranks any discovery status: leaving the
 * earlier message up would explain everything except why the button is dead.
 */
export function modelFieldStatus<T extends { message: string; tone: string }>({
  catalog,
  discoveryStatus,
  isTypedEntry,
  model,
}: {
  catalog: readonly { id: string }[] | null;
  discoveryStatus: T | null;
  isTypedEntry: boolean;
  model: string;
}): {
  blocked: boolean;
  status: T | { message: string; tone: "warning" } | null;
} {
  const error = typedModelCatalogError({ catalog, isTypedEntry, model });
  return {
    blocked: error !== null,
    status: error ? { message: error, tone: "warning" } : discoveryStatus,
  };
}

/**
 * Whether the Model control should render given discovery state.
 *
 * Optional-model harnesses (Claude Code / Codex, `acpNative`) omit the control
 * while discovery is in flight and after a **confirmed successful empty**
 * catalog (IPC resolved, no usable options) — there is nothing useful to pick.
 * Discovery failures / unavailable runtimes keep the control so #2246 failure
 * UI can render. Full disclosure still shows the control when Custom model is
 * available. Required-model harnesses always render the control.
 */
export function shouldRenderModelControl({
  discoveredModelOptions,
  modelDiscoveryLoading,
  modelDiscoverySuccessfulEmpty,
  modelIsOptional,
  showCustomModelOption,
}: {
  discoveredModelOptions: readonly { id: string }[] | null;
  modelDiscoveryLoading: boolean;
  /** True only when discovery IPC resolved with a response that yielded no options. */
  modelDiscoverySuccessfulEmpty: boolean;
  modelIsOptional: boolean;
  showCustomModelOption: boolean;
}): boolean {
  if (!modelIsOptional) return true;
  if (modelDiscoveryLoading) return false;
  const hasExplicitModel = (discoveredModelOptions ?? []).some(
    (option) => option.id.trim().length > 0,
  );
  if (hasExplicitModel) return true;
  if (showCustomModelOption) return true;
  // Omit only on confirmed successful empty — not on failure/unavailable.
  return !modelDiscoverySuccessfulEmpty;
}
