/** Runtime provider-capability tri-state used by the submit path. */
export type ProviderRuntimeCapability = "capable" | "locked" | "unknown";

/**
 * Classify a runtime id's provider-selection capability as a tri-state,
 * independent of whether the runtime catalog has loaded yet.
 *
 * The submit path keys its provider write on this: "capable" persists the
 * provider, "locked" clears it, and "unknown" OMITS the field so a transient
 * loading/error state (or a genuinely unknown/custom command) never becomes a
 * destructive write.
 *
 * Before the catalog loads, `prospectiveRuntimeId` can already be a known
 * persona runtime string (e.g. `buzz-agent`). A catalog lookup then returns
 * `undefined`, which would misclassify a provider-backed runtime as "unknown"
 * and omit the provider while still clearing the command override / writing env
 * — leaving the record inheriting a provider-backed runtime with a null
 * provider. To avoid that, we resolve capability STATICALLY for known ids:
 *
 * - buzz-agent / goose → "capable" (`isProviderCapable`, id-based).
 * - claude / codex → "locked" (CLI-login runtimes; no LLM provider selection).
 * - anything else (custom, empty, genuinely unknown) → "unknown".
 *
 * `isProviderCapable` is the caller-supplied {@link
 * runtimeSupportsLlmProviderSelection} result, kept as the single source of
 * truth for the capable set rather than re-hardcoding it here.
 */
export function resolveRuntimeProviderCapability(
  runtimeId: string,
  isProviderCapable: boolean,
): ProviderRuntimeCapability {
  if (isProviderCapable) {
    return "capable";
  }
  const id = runtimeId.trim();
  if (id === "claude" || id === "codex") {
    return "locked";
  }
  return "unknown";
}

export function shouldClearModelForRuntimeChange(
  previousRuntime: string,
  nextRuntime: string,
): boolean {
  const previous = previousRuntime.trim();
  const next = nextRuntime.trim();

  return previous.length > 0 && previous !== next;
}

/**
 * Resolve the `agentCommand` field to send on Save for the harness pin.
 *
 * The backend treats an empty string as the "inherit from persona" sentinel
 * (clears the override) and any concrete command as an explicit pin.
 * `undefined` means "leave the record's command alone".
 *
 * - Inheriting: send the sentinel only if there's a pin to clear, so a
 *   name-only edit leaves the record untouched.
 * - Pinning: normally send the command only when it diverges from the resolved
 *   value the dialog opened with, so an unchanged save stays a no-op. The
 *   exception is an inherit→pin transition (no override at open): the command
 *   field is prefilled with the resolved effective command, so accepting it
 *   as-is leaves it equal to `agentCommand` — without forcing the pin the
 *   update would be omitted and the agent would keep inheriting. An empty
 *   command never reaches the force branch (the caller blocks Save for an empty
 *   pinned custom command; catalog runtimes always set a concrete command).
 */
export function resolveAgentCommandUpdate(input: {
  inheritHarness: boolean;
  /** The command currently in the (possibly prefilled) input. */
  agentCommand: string;
  /** The resolved effective command the dialog opened with. */
  originalAgentCommand: string;
  /** The persisted override, or null when the agent was inheriting. */
  agentCommandOverride: string | null;
}): string | undefined {
  if (input.inheritHarness) {
    return input.agentCommandOverride != null ? "" : undefined;
  }
  const pinnedCommand = input.agentCommand.trim();
  const pinningFromInherit = input.agentCommandOverride == null;
  if (
    pinnedCommand !== input.originalAgentCommand ||
    (pinningFromInherit && pinnedCommand.length > 0)
  ) {
    return pinnedCommand;
  }
  return undefined;
}

/**
 * Whether any of the runtime/provider-required credential keys is unset.
 *
 * A key counts as missing when its env value is absent or an empty string
 * (matching {@link EnvVarsEditor}'s own `isMissing` rendering). The
 * `requiredEnvKeys` list is already filtered to keys the dialog can fix —
 * CLI-login runtimes (claude/codex) and keys satisfied by the runtime file
 * config contribute no entries, so this never blocks on out-of-band auth.
 */
export function hasMissingRequiredEnvKey(
  requiredEnvKeys: string[],
  envVars: Record<string, string>,
): boolean {
  return requiredEnvKeys.some((key) => (envVars[key] ?? "").length === 0);
}

/**
 * Resolve the provider and env-vars to PERSIST on Save.
 *
 * The spawn path reads ONLY the record snapshot (`record.provider`/
 * `record.env_vars`), never the live persona, and the record is authoritative:
 * `env_vars` is the complete pinned map (persona env snapshotted at create,
 * already merged UNDER the agent's own overrides), and the provider field is
 * user-editable in the dialog even while inheriting. So the local edit state IS
 * the record's own value and is honored verbatim in the normal case.
 *
 * The ONE exception is the inherit-TRANSITION-from-a-harness-pin: a previously
 * harness-pinned agent (e.g. Claude — `agent.agentCommandOverride != null` at
 * dialog open) has its `provider` cleared and carries no persona credential,
 * then the user checks "Inherit runtime from template" for a provider-backed
 * persona (e.g. buzz-agent/Anthropic). Persisting the local (empty) provider +
 * credential-less env would save an agent that fails readiness on next start.
 * Only in that case — inheriting AND the local provider is empty AND the agent
 * was harness-pinned at open — do we substitute the persona snapshot: the
 * persona's provider and the persona-layered env (`{ ...personaEnv,
 * ...agentEnv }`, agent layer wins to mirror spawn-time layering), matching
 * create-time record pinning.
 *
 * A NON-empty local provider while inheriting (e.g. an Anthropic-persona agent
 * the user re-points to Databricks) is a deliberate edit and passes through
 * unchanged — we never overwrite it with the persona provider.
 *
 * MODEL follows the same transition rule. buzz-agent/goose readiness requires a
 * model, but a Claude-pinned agent's record often carries no model (Claude
 * resolves its own). On the inherit-transition to a provider-backed persona with
 * a set `persona.model`, persisting the empty local model would save an agent
 * that inherits the provider + credentials but no model and fails readiness on
 * next start — so we substitute `personaModel` in that same case. A non-empty
 * local model (deliberate pick) always passes through; an empty local model in
 * steady state stays empty (runtime default), same authoritative logic.
 *
 * An EMPTY local provider while inheriting on an agent that was ALREADY
 * inheriting at open (`agentWasHarnessPinned` false) is ALSO authoritative: the
 * user either never set one or deliberately picked the "Default" option to
 * clear a saved override, so we persist `null` (runtime default) rather than
 * resurrect the persona provider — otherwise the Default option could never
 * actually clear an inherited agent's provider override.
 *
 * The result is the SAME effective value the required-credential gate
 * validates, so the gate, the submitted record, and the spawn snapshot agree.
 * Provider is normalized: trimmed, empty → `null`.
 */
export function resolveInheritedRuntimeSubmission(input: {
  inheritHarness: boolean;
  /**
   * Whether the agent was harness-pinned (`agentCommandOverride != null`) at
   * dialog open. Distinguishes the inherit-transition (was pinned, now
   * inheriting) from steady-state inherit (was already inheriting), so an empty
   * provider in steady state clears the override instead of resurrecting the
   * persona provider.
   */
  agentWasHarnessPinned: boolean;
  /** Local provider edit state (from the agent record, user-editable). */
  provider: string;
  /** The linked persona's provider, or empty when none/unset. */
  personaProvider: string;
  /** Local model edit state (from the agent record, user-editable). */
  model: string;
  /** The linked persona's model, or empty/null when none/unset. */
  personaModel: string | null;
  /** Local env-vars edit state (the agent's own layer). */
  envVars: Record<string, string>;
  /** The persona's env vars, layered under the agent's own on transition. */
  personaEnvVars: Record<string, string>;
}): {
  provider: string | null;
  model: string | null;
  envVars: Record<string, string>;
} {
  const localProvider = input.provider.trim();
  const localModel = input.model.trim();
  // Substitute the persona snapshot ONLY on the true inherit-transition: the
  // agent was harness-pinned at open, is now inheriting, and has an empty local
  // provider. Otherwise the local edit state is authoritative — a non-empty
  // provider is a deliberate pick, and an empty provider on an already-
  // inheriting agent is a deliberate clear (Default) — and passes through.
  if (
    input.inheritHarness &&
    input.agentWasHarnessPinned &&
    localProvider.length === 0
  ) {
    return {
      provider: input.personaProvider.trim() || null,
      // Fill an empty local model from the persona so a provider-backed runtime
      // isn't saved model-less; a deliberate local model still wins.
      model: localModel || input.personaModel?.trim() || null,
      envVars: { ...input.personaEnvVars, ...input.envVars },
    };
  }
  return {
    provider: localProvider || null,
    model: localModel || null,
    envVars: input.envVars,
  };
}

/** Inputs for {@link computeEditAgentFormValidity} — all pre-derived primitives. */
export interface EditAgentFormValidityInput {
  name: string;
  parallelism: string;
  turnTimeoutSeconds: string;
  /** The command already persisted on the agent (empty when inheriting). */
  agentAcpCommand: string;
  acpCommand: string;
  respondTo: string;
  respondToAllowlistLength: number;
  selectedRuntimeId: string;
  inheritHarness: boolean;
  agentCommand: string;
  /**
   * Whether a runtime/provider-required credential key is still unset. When
   * true the Save button is blocked — the agent would otherwise persist with a
   * missing credential and crash-loop on next start. See
   * {@link hasMissingRequiredEnvKey}.
   */
  requiredEnvKeyMissing: boolean;
}

/**
 * Pure field-validity check for the Edit Agent dialog's Save button.
 *
 * Mirrors the harness/backend validation so the user sees a disabled button
 * instead of a round-tripped error:
 * - name is required;
 * - parallelism / timeout must be blank or parseable integers;
 * - a previously-set ACP command cannot be cleared to empty (spawn failure);
 * - allowlist respond-to mode needs at least one entry;
 * - a pinned "Custom command" runtime (custom selection with inheritance
 *   cleared) must carry a concrete command — an empty command would spawn a
 *   runtime with no command.
 * - a runtime/provider-required credential key must be present — persisting
 *   with a missing key would crash-loop the agent on next start.
 */
export function computeEditAgentFormValidity(
  input: EditAgentFormValidityInput,
): boolean {
  const parallelismValid =
    input.parallelism.trim() === "" ||
    !Number.isNaN(Number.parseInt(input.parallelism, 10));
  const timeoutValid =
    input.turnTimeoutSeconds.trim() === "" ||
    !Number.isNaN(Number.parseInt(input.turnTimeoutSeconds, 10));
  const acpCommandValid = !(
    input.agentAcpCommand && input.acpCommand.trim() === ""
  );
  const respondToValid =
    input.respondTo !== "allowlist" || input.respondToAllowlistLength > 0;
  const customCommandValid = !(
    input.selectedRuntimeId === "custom" &&
    !input.inheritHarness &&
    input.agentCommand.trim() === ""
  );

  return (
    input.name.trim().length > 0 &&
    parallelismValid &&
    timeoutValid &&
    acpCommandValid &&
    respondToValid &&
    customCommandValid &&
    !input.requiredEnvKeyMissing
  );
}
