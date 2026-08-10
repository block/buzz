/**
 * Pure logic helpers for custom-harness deletion (blast-radius confirmation).
 *
 * Extracted for deterministic unit-testing — no React, no Tauri, no network.
 */

import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";

/**
 * Returns true iff the given catalog entry is editable by the user.
 * Only `source === "custom"` entries are editable/deletable.
 */
export function isEditableEntry(entry: AcpRuntimeCatalogEntry): boolean {
  return entry.source === "custom";
}

/**
 * Count managed agents whose effective harness is the given definition id —
 * either a direct record-level `runtime` pin, or inheritance from a linked
 * persona whose `runtime` is that id.
 *
 * Drives the delete-confirmation copy: deleting a harness that agents still
 * reference is allowed (blocking would turn cleanup into dependency
 * untangling), but the user is told those agents will stop launching.
 */
export function countAgentsReferencingHarness(
  harnessId: string,
  agents: ReadonlyArray<{ runtime: string | null; personaId: string | null }>,
  personas: ReadonlyArray<{ id: string; runtime: string | null }>,
): number {
  const personaRuntime = new Map(personas.map((p) => [p.id, p.runtime]));
  return agents.filter((agent) => {
    if (agent.runtime !== null) return agent.runtime === harnessId;
    if (agent.personaId === null) return false;
    return personaRuntime.get(agent.personaId) === harnessId;
  }).length;
}

/**
 * Confirmation copy for deleting a custom harness. Names the blast radius
 * when agents still reference the definition; stays quiet when none do.
 */
export function deleteHarnessConfirmMessage(
  label: string,
  referencingAgents: number,
): string {
  if (referencingAgents === 0) return `Delete ${label}?`;
  const noun = referencingAgents === 1 ? "agent uses" : "agents use";
  return `${referencingAgents} ${noun} this harness and will stop launching. Delete ${label}?`;
}

/** Minimal query-state shape consumed by `deleteConfirmState` — matches the
 * relevant subset of a TanStack `useQuery` result so the logic stays pure. */
export type BlastRadiusQuery<T> = {
  /** True during the initial fetch (no data yet). */
  isPending: boolean;
  isError: boolean;
  data: T | undefined;
};

export type DeleteConfirmState = {
  /** False while the blast radius is still unknown — the destructive button
   * must be disabled so a quick click can't beat the warning. */
  canConfirm: boolean;
  message: string;
};

/**
 * Drive the delete-confirmation UI from the two blast-radius queries.
 *
 * - While either query is initial-loading, confirmation is DISABLED and the
 *   copy says so: the promised "N agents will stop launching" warning must
 *   render before the destructive action is clickable.
 * - On query failure, deletion stays possible (blocking cleanup on a failed
 *   read would be worse) but the copy states the count is unknown — it never
 *   silently claims zero dependents.
 */
export function deleteConfirmState(
  harnessId: string,
  label: string,
  agents: BlastRadiusQuery<
    ReadonlyArray<{ runtime: string | null; personaId: string | null }>
  >,
  personas: BlastRadiusQuery<
    ReadonlyArray<{ id: string; runtime: string | null }>
  >,
): DeleteConfirmState {
  if (agents.isPending || personas.isPending) {
    return {
      canConfirm: false,
      message: "Checking which agents use this harness…",
    };
  }
  if (agents.isError || personas.isError) {
    return {
      canConfirm: true,
      message: `Couldn't check which agents use this harness — some agents may stop launching. Delete ${label}?`,
    };
  }
  const count = countAgentsReferencingHarness(
    harnessId,
    agents.data ?? [],
    personas.data ?? [],
  );
  return {
    canConfirm: true,
    message: deleteHarnessConfirmMessage(label, count),
  };
}
