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
 * When inheriting a persona's runtime, the runtime/provider/env that will
 * actually run come from the persona — but the agent record's own
 * `provider`/`envVars` may be stale (a previously harness-pinned agent can have
 * its provider cleared and carry no persona credential). The spawn path reads
 * ONLY the record snapshot (`record.provider`/`record.env_vars`), never the
 * live persona, so the persona snapshot must be persisted on the inherit
 * transition — otherwise the saved agent inherits e.g. buzz-agent/Anthropic
 * with no provider and no credential and fails readiness on next start. This
 * mirrors create-time, which pins the persona snapshot into the record.
 *
 * These are the SAME effective values the required-credential gate validates,
 * so the gate, the submitted record, and the spawn snapshot all agree.
 *
 * - `provider`: the persona's provider when inheriting, else the local edit
 *   state. Normalized: trimmed, empty → `null`.
 * - `envVars`: the persona-layered map (`{ ...personaEnv, ...agentEnv }`) when
 *   inheriting, else the local edit state. The agent's own layer wins, matching
 *   the spawn-time layering.
 *
 * When not inheriting, both pass through the local edit state unchanged.
 */
export function resolveInheritedRuntimeSubmission(input: {
  inheritHarness: boolean;
  /** Local provider edit state (from the agent record). */
  provider: string;
  /** The linked persona's provider, or empty when none/unset. */
  personaProvider: string;
  /** Local env-vars edit state (the agent's own layer). */
  envVars: Record<string, string>;
  /** The persona's env vars, layered under the agent's own on inherit. */
  personaEnvVars: Record<string, string>;
}): { provider: string | null; envVars: Record<string, string> } {
  if (!input.inheritHarness) {
    return {
      provider: input.provider.trim() || null,
      envVars: input.envVars,
    };
  }
  return {
    provider: input.personaProvider.trim() || null,
    envVars: { ...input.personaEnvVars, ...input.envVars },
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
