import { providerRecordHarness } from "@/features/agents/lib/pinnedHarness";
import type { AgentPersona, ManagedAgent } from "@/shared/api/types";

/** Which editor the profile panel's "Edit agent" action opens. */
export type ProfileEditAgentTarget = "instance" | "definition";

/**
 * Route the profile panel's Edit action to the editor that can actually show
 * this agent's configuration.
 *
 * Rule 20 (`features/agents/AGENTS.md`): a provider record answers from itself.
 * Its harness, command and model describe the HOST, and the definition
 * projection drops all three by design (`ManagedAgentRecord::to_definition_view`
 * has no slot for `backend` or `agent_command`), so the definition dialog opens
 * on a blank runtime and re-seeds it from this computer's catalog — showing a
 * remote agent as running a local harness it has never run. `providerRecordHarness`
 * stays the single owner of that question, so a local persona-backed agent takes
 * the definition path exactly as before.
 */
export function profileEditAgentTarget({
  managedAgent,
  resolvedPersona,
}: {
  managedAgent: ManagedAgent | undefined;
  resolvedPersona: AgentPersona | undefined;
}): ProfileEditAgentTarget {
  if (managedAgent && providerRecordHarness(managedAgent)) return "instance";
  return resolvedPersona ? "definition" : "instance";
}

/**
 * Whether the definition dialog this panel opens is editing a provider-backed
 * record, which must not have a local harness seeded into it.
 *
 * Edit-only, and the shape is what says so: this one dialog is driven by three
 * handlers, and Duplicate seeds a CREATE (no `id`) from the same
 * provider-backed profile an Edit would. Without the `id` check the guard shed
 * the create's harness while `useCreateRuntimeSeed`'s create-only effect
 * re-seeded it, and the two fought until React gave up.
 */
export function profileDialogEditsProviderRecord({
  initialValues,
  managedAgent,
}: {
  initialValues: object | null | undefined;
  managedAgent: ManagedAgent | undefined;
}): boolean {
  if (initialValues == null || !("id" in initialValues)) return false;
  return (
    managedAgent !== undefined && providerRecordHarness(managedAgent) !== null
  );
}
