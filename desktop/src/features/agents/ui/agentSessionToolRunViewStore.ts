import type { ToolRunGroupIdentityTable } from "./agentSessionToolRunGroupKey";
import type { ToolRunGroupViewState } from "./agentSessionToolRunViewState";

/**
 * Per-group presentation state, keyed by a group's stable identity.
 *
 * This lives outside React because a group's React identity is not stable
 * across regrouping: a same-kind run that later absorbs a differing tool call
 * becomes a mixed burst with a different summary id, which remounts the card.
 * Without this store, a reader's deliberate choice to open a group would be
 * silently reverted by the agent emitting one more tool call.
 *
 * Community-scoped like every other transcript cache — see
 * `resetCommunityState()` in `features/communities/useCommunityInit.ts`.
 */
const groupViewStates = new Map<string, ToolRunGroupViewState>();

/**
 * Leaf-key → group-identity tables, one per transcript scope.
 *
 * Group identity is resolved by recognising leaves carried over from the
 * previous grouping pass (see `resolveToolRunGroupIdentities`), so the
 * recognition table has to outlive a render. It is scoped per transcript rather
 * than shared globally because two transcripts can be mounted at once (a
 * channel pane plus a compact preview) and each resolves only the groups it
 * renders — a shared table would let one panel forget the other's leaves.
 */
const groupIdentityTables = new Map<string, Map<string, string>>();

const emptyIdentityTable: ToolRunGroupIdentityTable = new Map<string, string>();

export function readToolRunGroupViewState(
  groupKey: string,
): ToolRunGroupViewState | undefined {
  return groupViewStates.get(groupKey);
}

export function writeToolRunGroupViewState(
  groupKey: string,
  state: ToolRunGroupViewState,
): void {
  groupViewStates.set(groupKey, state);
}

export function readToolRunGroupIdentityTable(
  scope: string,
): ToolRunGroupIdentityTable {
  return groupIdentityTables.get(scope) ?? emptyIdentityTable;
}

export function writeToolRunGroupIdentityTable(
  scope: string,
  table: Map<string, string>,
): void {
  groupIdentityTables.set(scope, table);
}

/** Clear all remembered group state (community switch / test isolation). */
export function resetAgentSessionToolRunViewState(): void {
  groupViewStates.clear();
  groupIdentityTables.clear();
}
