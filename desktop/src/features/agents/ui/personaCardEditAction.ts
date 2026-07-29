import { providerRecordHarness } from "@/features/agents/lib/pinnedHarness";
import type { AgentPersona, ManagedAgent } from "@/shared/api/types";

/** The record an agent card's Edit opens, plus the definition behind it. */
export type PersonaCardInstanceEdit = {
  agent: ManagedAgent;
  /**
   * The definition this record inherits its shared identity from, or `null`
   * when nothing about it is editable here. The instance dialog shows the
   * avatar but cannot change it — it is definition-level identity — so this is
   * what its "Edit avatar" hand-off needs, exactly as the profile panel's
   * mount of the same dialog passes `resolvedPersona`.
   */
  persona: AgentPersona | null;
};

/**
 * Which editor an agent card's "Edit" action opens, and for what.
 *
 * One action rather than two booleans, because the two editors are driven by
 * two pieces of state (`agentToEditInstance`, `personaDialogState`) and a
 * caller that could set both would mount both dialogs at once. A single
 * discriminated result makes the exclusivity structural.
 */
export type PersonaCardEditAction =
  | ({ type: "instance" } & PersonaCardInstanceEdit)
  | { type: "definition" };

/**
 * Route the agents card's Edit action to the editor that can actually show
 * this agent's configuration.
 *
 * Rule 19 (`features/agents/AGENTS.md`), on the third door into the same
 * dialog: a provider record answers from itself. Its harness, command and
 * model describe the HOST, and the definition projection carries none of them
 * (`ManagedAgentRecord::to_definition_view` has no slot for `backend` or
 * `agent_command`), so the definition dialog would open on a blank runtime and
 * re-seed it from this computer's catalog. `providerRecordHarness` stays the
 * single owner of that question, so a local persona-backed agent — and a card
 * with no linked record at all — takes the definition path exactly as before.
 *
 * The record is the one the card itself renders (`pickProfileAgent`), so a
 * persona with several instances edits the record the user is looking at
 * rather than a guess. That is why this door does not refuse on ambiguity the
 * way `agentManagementUpdateTarget` does: that one resolves a name out of a
 * chat message with no visual referent, this one has the card.
 *
 * Routing away from the definition dialog must not cost the user the
 * definition, so a non-built-in persona rides along for the instance dialog's
 * "Edit avatar" hand-off. A built-in one does not: its definition is not
 * editable, and the definition dialog would not have offered that either.
 */
export function personaCardEditAction(
  persona: AgentPersona,
  linkedAgent: ManagedAgent | undefined,
): PersonaCardEditAction {
  if (linkedAgent && providerRecordHarness(linkedAgent)) {
    return {
      type: "instance",
      agent: linkedAgent,
      persona: persona.isBuiltIn ? null : persona,
    };
  }
  return { type: "definition" };
}
