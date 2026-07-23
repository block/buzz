import type { AgentAiConfigurationMode } from "./agentAiConfigurationPolicy";

/**
 * Inputs for {@link personaSubmitBlock}. Every field is an OUTPUT of a gate the
 * dialog already computes for `canSubmit` — this module maps those outputs to a
 * single human-readable reason. It must not recompute policy: the derivation
 * stays a pure function of the gate results so the message can never disagree
 * with whether the button is actually disabled.
 */
export type PersonaSubmitBlockInput = {
  /** A save/create request is in flight (button shows "Saving..."). */
  isPending: boolean;
  /** The avatar upload is in flight (button shows "Uploading..."). */
  isAvatarUploadPending: boolean;
  /** Trimmed display name is empty. */
  displayNameEmpty: boolean;
  /** Create (new definition) vs edit (existing). Some gates are create-only. */
  isCreateMode: boolean;
  /** A runtime has been chosen (create-only gate). */
  runtimeChosen: boolean;
  /** The chosen runtime is available on this machine (create-only gate). */
  runtimeAvailable: boolean;
  /** The remote / where-to-run backend selection is incomplete (create-only). */
  createBackendBlocked: boolean;
  /** Respond-to allowlist mode is selected but the allowlist is empty. */
  allowlistEmpty: boolean;
  /** Selected AI configuration mode: inherit global defaults vs customize. */
  aiConfigurationMode: AgentAiConfigurationMode;
  /** `computeLocalModeGate(...).satisfied` — resolved AI config is complete. */
  localModeSatisfied: boolean;
  /** `computeLocalModeGate(...).missingNormalizedFields`, e.g. ["provider"]. */
  localModeMissingFields: readonly string[];
  /** `computeLocalModeGate(...).missingEnvKeys` — required credentials unset. */
  localModeMissingEnvKeys: readonly string[];
  /** `agentAiConfigurationModeSatisfied(...)` for the Customize pair. */
  customAiPairSatisfied: boolean;
  /** Runtime exposes a provider picker (Buzz Agent / Goose), not Codex/Claude. */
  runtimeNeedsProviderSelection: boolean;
  /** Customize provider field is empty. */
  customProviderEmpty: boolean;
  /** Customize model field is empty. */
  customModelEmpty: boolean;
};

function joinWithAnd(parts: readonly string[]): string {
  if (parts.length <= 1) return parts[0] ?? "";
  if (parts.length === 2) return `${parts[0]} and ${parts[1]}`;
  return `${parts.slice(0, -1).join(", ")}, and ${parts[parts.length - 1]}`;
}

/**
 * Describe the concrete missing pieces behind an unsatisfied AI-config gate,
 * naming the actual fix rather than a generic "configuration incomplete".
 */
function describeMissingAiPieces(
  fields: readonly string[],
  envKeys: readonly string[],
): string {
  const parts: string[] = [];
  if (fields.includes("provider")) parts.push("a provider");
  if (fields.includes("model")) parts.push("a model");
  for (const key of envKeys) parts.push(`a value for ${key}`);
  return joinWithAnd(parts);
}

/**
 * Human-readable reason the Create/Save button is disabled, or `null` when the
 * form can be submitted. Precedence mirrors the `canSubmit` term order in
 * AgentDefinitionDialog so the surfaced reason is deterministic and always the
 * first blocking input — correcting it makes the reason advance or disappear.
 *
 * While a request or avatar upload is in flight the button communicates the
 * progress itself ("Saving..." / "Uploading..."), so no reason is returned.
 */
export function personaSubmitBlock(
  input: PersonaSubmitBlockInput,
): string | null {
  if (input.isPending || input.isAvatarUploadPending) {
    return null;
  }

  // 1. Required definition fields.
  if (input.displayNameEmpty) {
    return "Enter a name for this agent.";
  }

  // 2–4. Create-only runtime / backend gates.
  if (input.isCreateMode) {
    if (!input.runtimeChosen) {
      return "Choose where this agent runs.";
    }
    if (!input.runtimeAvailable) {
      return "The selected runtime isn't available on this machine.";
    }
    if (input.createBackendBlocked) {
      return "Finish configuring the remote backend before creating this agent.";
    }
  }

  // 5. Access / allowlist crash-loop guard (create and edit).
  if (input.allowlistEmpty) {
    return "Add at least one allowed sender, or change who this agent responds to.";
  }

  // 6. Resolved AI configuration (provider/model/credentials) incomplete.
  if (!input.localModeSatisfied) {
    const missing = describeMissingAiPieces(
      input.localModeMissingFields,
      input.localModeMissingEnvKeys,
    );
    if (input.aiConfigurationMode === "defaults") {
      const detail = missing ? ` — missing ${missing}` : "";
      return `Your global AI defaults are incomplete${detail}. Set them in Settings → AI defaults, or choose Customize to configure this agent directly.`;
    }
    return missing
      ? `This agent's AI configuration is missing ${missing}.`
      : "Complete this agent's AI configuration.";
  }

  // 7. Customize pair incomplete (form provider/model empty while a global
  // fallback keeps localMode satisfied). Provider only counts where the runtime
  // exposes a picker — Codex/Claude drive their own provider.
  if (!input.customAiPairSatisfied) {
    const needProvider =
      input.runtimeNeedsProviderSelection && input.customProviderEmpty;
    const pieces: string[] = [];
    if (needProvider) pieces.push("a provider");
    if (input.customModelEmpty) pieces.push("a model");
    const what =
      pieces.length > 0 ? joinWithAnd(pieces) : "the AI configuration";
    const defaultsLabel = input.runtimeNeedsProviderSelection
      ? "Use AI defaults"
      : "Use harness defaults";
    return `Select ${what} for this agent, or switch to ${defaultsLabel}.`;
  }

  return null;
}
