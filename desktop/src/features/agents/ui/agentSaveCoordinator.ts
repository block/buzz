/**
 * agentSaveCoordinator.ts — Save coordinator per Artifact 3 of the Phase 0 spec.
 *
 * Execution sequence on every submit:
 *   0. Validate (pre-write):  runtime availability, credential gate,
 *                             parallelism parsing
 *   1. Definition write       iff a D-field changed AND definition is editable
 *   2. Instance write         iff an I-field changed (row 1/8 drops excluded)
 *   3. Local-policy setters   only on change
 *   4. Settlement             re-fetch BOTH stores, derive persisted-vs-unsaved
 *                             from observed state (not from command result)
 *
 * Partial-failure: stop at first failure, run settlement, toast from observed
 * state — naming exactly what persisted and what didn't.
 *
 * Not in scope: 14a membership, 14b identity archive — no write path through Save.
 */

import { toast } from "sonner";
import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import { showAgentProfileSyncWarning } from "@/features/agents/ui/agentProfileSyncWarning";
import { personaSaveNotice } from "@/features/agents/lib/personaSaveNotice";
import { validateLinkedAgentRuntimeEdit } from "@/features/profile/ui/UserProfilePanelPersonaSubmit";
import type {
  AcpRuntimeCatalogEntry,
  AgentPersona,
  ManagedAgent,
  UpdateManagedAgentInput,
  UpdatePersonaInput,
} from "@/shared/api/types";
import type { PersonaSharePublicationResult } from "@/shared/api/tauriPersonas";
import type { AgentEditContext } from "./agentFormModel";
import {
  editContextDefinition,
  editContextInstance,
  envVarsMapEqual,
  namePoolEqual,
} from "./agentFormModel";

// ── Types ────────────────────────────────────────────────────────────────────

export type SaveCoordinatorOptions = {
  /** Current edit context (definition + optional instance). */
  ctx: AgentEditContext;
  /** Submitted definition update (null = no D-change or team-managed). */
  personaInput: UpdatePersonaInput | null;
  /** Submitted instance update (null = no I-change). */
  agentInput: UpdateManagedAgentInput | null;
  /** Policy setter calls. */
  policySets: Array<
    | { type: "autoRestart"; pubkey: string; value: boolean }
    | { type: "startOnAppLaunch"; pubkey: string; value: boolean }
  >;
  /** Whether the submitted definition was flagged for catalog publish. */
  publishCatalogUpdates?: boolean;
  /** Validated runtime catalog for the runtime-edit gate. */
  runtimes?: readonly AcpRuntimeCatalogEntry[];

  // Mutations
  updatePersona: (input: UpdatePersonaInput) => Promise<unknown>;
  updatePersonaAndPublish: (
    input: UpdatePersonaInput,
  ) => Promise<PersonaSharePublicationResult>;
  updateManagedAgent: (
    input: UpdateManagedAgentInput,
  ) => Promise<{ agent: ManagedAgent; profileSyncError: string | null }>;
  setAutoRestart: (pubkey: string, value: boolean) => Promise<unknown>;
  setStartOnAppLaunch: (pubkey: string, value: boolean) => Promise<unknown>;

  /** Re-fetch both stores and return the current (persona, agent) pair. */
  refetchStores: () => Promise<{
    persona: AgentPersona | null;
    agent: ManagedAgent | null;
  }>;

  /** Called when the dialog should close (on full success). */
  onDone: () => void;
  /** Called after save to optionally offer "Start now" (passed the post-settle agent). */
  onSavedWhileStopped?: (agent: ManagedAgent) => void;
};

// ── Coordinator ───────────────────────────────────────────────────────────────

/**
 * Execute the Artifact 3 save sequence and settle from observed state.
 *
 * Returns true if all writes succeeded; false on partial or full failure.
 * Callers must keep the dialog open on false so the user sees the error
 * and the re-seeded form shows the unsaved remainder.
 */
export async function runAgentSaveCoordinator(
  opts: SaveCoordinatorOptions,
): Promise<boolean> {
  const {
    ctx,
    personaInput,
    agentInput,
    policySets,
    publishCatalogUpdates,
    runtimes,
    updatePersona,
    updatePersonaAndPublish,
    updateManagedAgent,
    setAutoRestart,
    setStartOnAppLaunch,
    refetchStores,
    onDone,
    onSavedWhileStopped,
  } = opts;

  const def = editContextDefinition(ctx);
  const inst = editContextInstance(ctx);

  // ── Step 0: Validate ──────────────────────────────────────────────────────
  if (personaInput && def && inst) {
    const runtimeError = validateLinkedAgentRuntimeEdit({
      input: personaInput,
      managedAgent: inst,
      previousPersona: def,
      runtimes: runtimes ? [...runtimes] : undefined,
    });
    if (runtimeError) {
      toast.error(runtimeError);
      return false;
    }
  }

  // ── Steps 1–3: Writes ─────────────────────────────────────────────────────
  let firstError: string | null = null;
  let profileSyncError: string | null = null;
  let latestAgent: ManagedAgent | null = inst;
  // Track publication status for the success toast.
  let publicationStatus:
    | PersonaSharePublicationResult["publicationStatus"]
    | null = null;

  // Per-policy failure tracking: track which policies succeeded individually.
  const policyResults: Array<{
    policy: (typeof policySets)[number];
    written: boolean;
  }> = [];

  // Step 1: Definition write
  if (personaInput && !firstError) {
    try {
      if (publishCatalogUpdates) {
        const result = await updatePersonaAndPublish(personaInput);
        publicationStatus = result.publicationStatus;
      } else {
        await updatePersona(personaInput);
      }
    } catch (err) {
      firstError =
        err instanceof Error ? err.message : "Failed to save agent profile.";
    }
  }

  // Step 2: Instance write
  if (agentInput && !firstError) {
    try {
      const result = await updateManagedAgent(agentInput);
      latestAgent = result.agent;
      profileSyncError = result.profileSyncError;
    } catch (err) {
      firstError =
        err instanceof Error ? err.message : "Failed to save agent settings.";
    }
  }

  // Step 3: Policy setters — run each independently, stop at first failure.
  if (!firstError) {
    for (const policy of policySets) {
      try {
        if (policy.type === "autoRestart") {
          await setAutoRestart(policy.pubkey, policy.value);
        } else {
          await setStartOnAppLaunch(policy.pubkey, policy.value);
        }
        policyResults.push({ policy, written: true });
      } catch (err) {
        policyResults.push({ policy, written: false });
        firstError =
          err instanceof Error ? err.message : "Failed to save agent policy.";
        break;
      }
    }
  }

  // ── Step 4: Settlement — re-fetch both stores ─────────────────────────────
  // Always runs (success or error) so the toast and retry-remainder are derived
  // from actual observed state, not from command-level booleans.
  const { persona: observedPersona, agent: observedAgent } =
    await refetchStores();

  // ── Derive what persisted from observed state ─────────────────────────────
  // Both success and error paths settle from observed state — command result
  // booleans are never the authority. The observed remainder is the set of
  // submitted changes that did not reach the store.
  const persistedParts: string[] = [];
  const failedParts: string[] = [];

  if (personaInput) {
    // Absent entity after re-fetch = not persisted.
    const persisted =
      observedPersona !== null &&
      observedStateMatchesPersonaInput(observedPersona, personaInput);
    if (persisted) {
      persistedParts.push("profile");
    } else {
      failedParts.push("profile");
    }
  }

  if (agentInput) {
    // Absent entity after re-fetch = not persisted.
    const persisted =
      observedAgent !== null &&
      observedStateMatchesAgentInput(observedAgent, agentInput);
    if (persisted) {
      persistedParts.push("instance settings");
    } else {
      failedParts.push("instance settings");
    }
  }

  // Per-policy observed check: track each policy individually.
  for (const { policy, written } of policyResults) {
    if (!written) {
      failedParts.push(
        policy.type === "autoRestart" ? "auto-restart policy" : "launch policy",
      );
    }
  }
  // Policies not yet attempted (stopped early at an error) also failed.
  const attemptedCount = policyResults.length;
  for (let i = attemptedCount; i < policySets.length; i++) {
    const policy = policySets[i];
    if (policy) {
      failedParts.push(
        policy.type === "autoRestart" ? "auto-restart policy" : "launch policy",
      );
    }
  }

  const observedRemainder = failedParts.length > 0;

  if (!observedRemainder) {
    // Full success — every submitted write is reflected in observed state.
    const agentName =
      latestAgent?.name ?? observedAgent?.name ?? def?.displayName ?? "Agent";

    if (profileSyncError) {
      showAgentProfileSyncWarning(agentName, profileSyncError);
    } else if (publishCatalogUpdates && personaInput) {
      // Use personaSaveNotice for the publish-specific success message.
      toast.success(personaSaveNotice(agentName, publicationStatus));
    } else {
      toast.success(`${agentName} saved.`);
    }

    // "Saved while stopped" affordance (Artifact 3 contract)
    const finalAgent = observedAgent ?? latestAgent;
    if (finalAgent && !isManagedAgentActive(finalAgent)) {
      onSavedWhileStopped?.(finalAgent);
    }

    onDone();
    return true;
  }

  // ── Partial or full failure — settle from observed state ──────────────────
  if (persistedParts.length > 0 && failedParts.length > 0) {
    const kept = persistedParts.join(" and ");
    const failed = failedParts.join(" and ");
    toast.warning(
      `${capitalizeFirst(kept)} saved. ${capitalizeFirst(failed)} failed: ${firstError ?? "not persisted"} — reopen to retry; your ${kept} change is kept.`,
    );
  } else if (persistedParts.length > 0) {
    toast.success(
      `Saved. Some changes may not have persisted — reopen to retry.`,
    );
  } else {
    toast.error(firstError ?? "Failed to save agent.");
  }

  return false;
}

// ── Observed-state comparison helpers ────────────────────────────────────────
// Used to determine what persisted after a partial failure. Compares
// CANONICAL stored values (trimmed strings, normalized name pool, map equality)
// per the settlement contract.

function observedStateMatchesPersonaInput(
  observed: AgentPersona,
  submitted: UpdatePersonaInput,
): boolean {
  // Required fields
  if (observed.displayName.trim() !== submitted.displayName.trim())
    return false;
  if (observed.systemPrompt.trim() !== (submitted.systemPrompt ?? "").trim())
    return false;
  // Optional fields — only compare when submitted
  if (
    submitted.avatarUrl !== undefined &&
    (observed.avatarUrl ?? "") !== (submitted.avatarUrl ?? "")
  )
    return false;
  if (
    submitted.runtime !== undefined &&
    (observed.runtime ?? null) !== (submitted.runtime ?? null)
  )
    return false;
  if ((observed.model ?? null) !== (submitted.model ?? null)) return false;
  if ((observed.provider ?? null) !== (submitted.provider ?? null))
    return false;
  if (!namePoolEqual(observed.namePool, submitted.namePool ?? [])) return false;
  if (!envVarsMapEqual(observed.envVars, submitted.envVars ?? {})) return false;
  // Behavior: compare respondTo/allowlist/parallelism when submitted
  if (submitted.behavior !== undefined) {
    const b = submitted.behavior;
    if (
      b.respondTo !== undefined &&
      (observed.respondTo ?? null) !== (b.respondTo ?? null)
    )
      return false;
    if (
      b.respondToAllowlist !== undefined &&
      observed.respondToAllowlist.join(",") !==
        (b.respondToAllowlist ?? []).join(",")
    )
      return false;
    if (
      b.parallelism !== undefined &&
      (observed.parallelism ?? null) !== (b.parallelism ?? null)
    )
      return false;
  }
  return true;
}

function observedStateMatchesAgentInput(
  observed: ManagedAgent,
  submitted: UpdateManagedAgentInput,
): boolean {
  // Only check fields that were submitted (undefined means "don't touch")
  if (
    submitted.name !== undefined &&
    (submitted.name ?? "").trim() !== observed.name.trim()
  ) {
    return false;
  }
  if (
    submitted.systemPrompt !== undefined &&
    (submitted.systemPrompt ?? null) !== (observed.systemPrompt ?? null)
  ) {
    return false;
  }
  if (
    submitted.model !== undefined &&
    (submitted.model ?? null) !== (observed.model ?? null)
  ) {
    return false;
  }
  if (
    submitted.provider !== undefined &&
    (submitted.provider ?? null) !== (observed.provider ?? null)
  ) {
    return false;
  }
  if (
    submitted.envVars !== undefined &&
    !envVarsMapEqual(submitted.envVars, observed.envVars)
  ) {
    return false;
  }
  if (
    submitted.respondTo !== undefined &&
    (submitted.respondTo ?? null) !== (observed.respondTo ?? null)
  ) {
    return false;
  }
  if (
    submitted.respondToAllowlist !== undefined &&
    submitted.respondToAllowlist.join(",") !==
      observed.respondToAllowlist.join(",")
  ) {
    return false;
  }
  if (
    submitted.parallelism !== undefined &&
    (submitted.parallelism ?? null) !== (observed.parallelism ?? null)
  ) {
    return false;
  }
  return true;
}

function capitalizeFirst(s: string): string {
  if (!s) return s;
  return s.charAt(0).toUpperCase() + s.slice(1);
}

// Re-export for convenient use in AgentEditDialog
export { seedAgentFormModel, emitAgentFormDiff } from "./agentFormModel";
export type { AgentFormModel, AgentEditContext } from "./agentFormModel";
