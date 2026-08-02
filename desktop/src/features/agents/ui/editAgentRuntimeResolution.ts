import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
import {
  getDefaultPersonaRuntime,
  runtimeSupportsLlmProviderSelection,
} from "./agentConfigOptions";

/**
 * Which runtime the Edit Agent dialog is really talking about.
 *
 * Extracted from the dialog because two independent consumers must agree on the
 * answer — the block-save credential gate and the submit path — and because the
 * remote case below is a rule about records, not about a component.
 */

/** Find a catalog entry by resolved command first, then by id. */
function matchRuntime(
  runtimes: readonly AcpRuntimeCatalogEntry[],
  command: string,
): AcpRuntimeCatalogEntry | undefined {
  const trimmed = command.trim();
  return (
    runtimes.find((runtime) => runtime.command?.trim() === trimmed) ??
    runtimes.find((runtime) => runtime.id === trimmed)
  );
}

/**
 * The catalog entry the harness dropdown should preselect, or `null` for none.
 *
 * A provider-backed record always answers `null`. Its harness runs on the HOST,
 * so a local match is either absent or a name collision — `claude-agent-acp`
 * happens to be a local builtin's command, which is the only reason a remote
 * Claude agent's dropdown ever looked right, and it would go on to point local
 * model discovery at this computer's Claude and present its models as the
 * host's.
 */
export function resolveDialogRuntimeId(
  runtimes: readonly AcpRuntimeCatalogEntry[],
  agentCommand: string,
  isProviderRecord: boolean,
): string | null {
  if (isProviderRecord) return null;
  return matchRuntime(runtimes, agentCommand)?.id ?? null;
}

/**
 * Whether the runtime the dialog OPENED with supports LLM provider selection.
 *
 * Edit-state runtime ids mutate during selection changes and so cannot identify
 * the original state; this resolves from the command the dialog opened with.
 */
export function resolveOriginalRuntimeSupportsProvider(
  runtimes: readonly AcpRuntimeCatalogEntry[],
  originalAgentCommand: string,
): boolean {
  return runtimeSupportsLlmProviderSelection(
    matchRuntime(runtimes, originalAgentCommand)?.id ?? "",
  );
}

/**
 * The runtime id that will actually be active after submit.
 *
 * A provider-backed record answers from its own pin and stops there. Its
 * harness runs on the HOST, so this computer's catalog cannot identify it: the
 * lookups below would miss (leaving the local default runtime — the "Buzz
 * Agent" lie) or hit by pure name collision, and either answer would then
 * decide which credentials the record needs and whether its provider is safe to
 * write. `""` for a pin this app does not recognize is the honest answer, the
 * same one `createGateHarnessId` gives for an unpinned remote harness: no known
 * harness to ask credential questions of.
 *
 * For a LOCAL record the resolution is unchanged:
 * - pinned: the selected catalog entry (or the raw selection id);
 * - inheriting: the LINKED DEFINITION's runtime — that is what will run once the
 *   override is cleared. Deriving from the record's command would be wrong for a
 *   pinned agent that just checked "Inherit runtime from template": the override
 *   is still on the record, so it would resolve to the old pin and hide the
 *   inherited runtime's required credentials.
 * - falling back to the record's command, then to the app default runtime, so
 *   discovery can still run for an agent whose definition has no runtime set.
 */
export function resolveProspectiveRuntimeId(input: {
  runtimes: readonly AcpRuntimeCatalogEntry[];
  /**
   * The harness id this record pins on the host, `""` when the pin is not one
   * this app recognizes, and `null` when the record runs on this computer.
   */
  pinnedRuntimeId: string | null;
  inheritHarness: boolean;
  /** The linked definition's runtime, or null/empty when unset. */
  personaRuntimeId: string | null | undefined;
  /** The record's resolved effective command. */
  agentCommand: string;
  selectedRuntimeId: string;
}): string {
  if (input.pinnedRuntimeId !== null) {
    return input.pinnedRuntimeId;
  }
  if (!input.inheritHarness) {
    return (
      input.runtimes.find((runtime) => runtime.id === input.selectedRuntimeId)
        ?.id ?? input.selectedRuntimeId
    );
  }
  const personaRuntimeId = input.personaRuntimeId?.trim();
  if (personaRuntimeId) {
    return (
      input.runtimes.find((runtime) => runtime.id === personaRuntimeId)?.id ??
      personaRuntimeId
    );
  }
  return (
    matchRuntime(input.runtimes, input.agentCommand)?.id ??
    getDefaultPersonaRuntime(input.runtimes)?.id ??
    ""
  );
}
