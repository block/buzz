import {
  providerRecordHarness,
  resolvePinnedHarness,
} from "@/features/agents/lib/pinnedHarness";
import type { ManagedAgent } from "@/shared/api/types";

/**
 * How the profile surfaces name the harness behind an agent.
 *
 * One owner for a rule that was typed twice — the panel's "Runtime" field and
 * the popover's badge carried byte-identical copies of the label table, so a
 * name learned in one place was still wrong in the other.
 */

/**
 * The friendly name for a command string a NON-record surface carries.
 *
 * The inputs here are a relay agent's self-declared `agentType` and a
 * definition's `runtime` preference: free-form strings from elsewhere rather
 * than a record's pin. They are still commands, so they are read by the one
 * command→harness owner (`resolvePinnedHarness`) instead of a second label
 * table — a rival table names `codex-acp` today and misses whatever the Rust
 * catalog learns tomorrow, with no mirror test to catch the gap.
 *
 * Args are empty because these surfaces carry a bare command, so nothing here
 * narrows by profile; an unrecognized command falls through to itself, which is
 * the honest answer for a name only its author knows. Every call site guards on
 * the string being non-empty, and one that is only whitespace reads "Not
 * configured" rather than rendering an invisible badge.
 */
export function runtimeCommandLabel(command: string): string {
  return resolvePinnedHarness(command, []).label;
}

/**
 * The harness label for a managed record.
 *
 * A provider-backed record answers from its own pin: the table above cannot
 * name a binary on the HOST, and its misses rendered a raw `hermes` where a
 * name belonged — with no way to tell two profiles of one harness apart, since
 * the profile lives in the args. A LOCAL record keeps resolving exactly as it
 * did; the catalog genuinely describes this computer.
 */
export function managedAgentRuntimeLabel(
  agent: Pick<ManagedAgent, "backend" | "agentCommand" | "agentArgs">,
): string {
  return (
    providerRecordHarness(agent)?.label ??
    runtimeCommandLabel(agent.agentCommand)
  );
}

/**
 * What copying the harness field yields: the pin as it runs on the host,
 * args included, so a pasted command is one a human can actually run there.
 */
export function managedAgentRuntimeCopyValue(
  agent: Pick<ManagedAgent, "backend" | "agentCommand" | "agentArgs">,
): string {
  return providerRecordHarness(agent)?.command ?? agent.agentCommand;
}
