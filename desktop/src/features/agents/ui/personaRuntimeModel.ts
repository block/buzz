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
    customCommandValid
  );
}
