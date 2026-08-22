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

  // ── Steps 1–3: Writes (per-boundary settlement) ───────────────────────────
  // Contract: settle after EACH attempted boundary. Stop advancing when the
  // preceding step did not persist to disk (observed-state check, not command
  // result). Never trust `written` booleans for policy success.
  let firstError: string | null = null;
  let profileSyncError: string | null = null;
  let latestAgent: ManagedAgent | null = inst;
  // Track publication status for the success toast.
  let publicationStatus:
    | PersonaSharePublicationResult["publicationStatus"]
    | null = null;

  // Step 1: Definition write — settle immediately, stop if not persisted.
  if (personaInput) {
    let caughtError: string | null = null;
    try {
      if (publishCatalogUpdates) {
        const result = await updatePersonaAndPublish(personaInput);
        publicationStatus = result.publicationStatus;
      } else {
        await updatePersona(personaInput);
      }
    } catch (err) {
      caughtError =
        err instanceof Error ? err.message : "Failed to save agent profile.";
    }
    // Settle after definition write — re-fetch regardless of throw. Observed
    // persistence (not the command result) decides whether the step failed: a
    // throw whose write is on disk is NOT a failed step, and a silent no-op
    // that did not persist IS. Only a genuine non-persisted write stops advance.
    const { persona: settled } = await refetchStores();
    const persisted =
      settled !== null &&
      observedStateMatchesPersonaInput(settled, personaInput);
    if (!persisted) {
      firstError =
        caughtError ?? "Agent profile did not persist — reopen to retry.";
    }
  }

  // Step 2: Instance write — only if definition step passed.
  if (agentInput && !firstError) {
    let caughtError: string | null = null;
    try {
      const result = await updateManagedAgent(agentInput);
      latestAgent = result.agent;
      profileSyncError = result.profileSyncError;
    } catch (err) {
      caughtError =
        err instanceof Error ? err.message : "Failed to save agent settings.";
    }
    // Settle after instance write — re-fetch regardless of throw; observed
    // persistence decides (a throw whose write persisted is not a failure).
    const { agent: settled } = await refetchStores();
    const persisted =
      settled !== null && observedStateMatchesAgentInput(settled, agentInput);
    if (!persisted) {
      firstError =
        caughtError ?? "Agent settings did not persist — reopen to retry.";
    }
  }

  // Step 3: Policy setters — run each independently, settle after each,
  // stop at first policy that did not observe as persisted.
  const policyResults: Array<{
    policy: (typeof policySets)[number];
    written: boolean;
  }> = [];
  if (!firstError) {
    for (const policy of policySets) {
      let caughtError: string | null = null;
      try {
        if (policy.type === "autoRestart") {
          await setAutoRestart(policy.pubkey, policy.value);
        } else {
          await setStartOnAppLaunch(policy.pubkey, policy.value);
        }
      } catch (err) {
        caughtError =
          err instanceof Error ? err.message : "Failed to save agent policy.";
      }
      // Settle after the setter — re-fetch regardless of throw, mirroring the
      // D/I steps above. Observed persistence (not the command result) decides:
      // both Tauri setters save the record BEFORE building their returned
      // summary, so a post-save summary error is a thrown-but-persisted write
      // and must NOT stop the sequence. Only an observed non-persist does.
      const { agent: settled } = await refetchStores();
      const persisted =
        settled !== null && observedPolicyMatches(settled, policy);
      policyResults.push({ policy, written: persisted });
      if (!persisted) {
        firstError =
          caughtError ??
          `${policy.type === "autoRestart" ? "Auto-restart" : "Start on launch"} policy did not persist.`;
        break;
      }
    }
  }

  // ── Step 4: Final settlement — re-fetch both stores ──────────────────────
  // Runs after all writes (success or partial/full error). Per-boundary
  // settlement above stopped advancing on any observed mismatch; this final
  // refetch establishes the definitive post-write state for toasts and retry.
  const { persona: observedPersona, agent: observedAgent } =
    await refetchStores();

  // ── Derive what persisted from observed state ─────────────────────────────
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

  // Per-policy observed check: use observed state from the final refetch,
  // not the policyResults.written boolean (which was set by per-boundary checks).
  // Policies not in policyResults were not attempted (stopped early).
  for (const policy of policySets) {
    const result = policyResults.find((r) => r.policy === policy);
    if (!result) {
      // Not attempted.
      failedParts.push(
        policy.type === "autoRestart" ? "auto-restart policy" : "launch policy",
      );
    } else {
      // Settle from observed agent (not written boolean) if available.
      const observedPersisted =
        observedAgent !== null && observedPolicyMatches(observedAgent, policy);
      if (observedPersisted) {
        persistedParts.push(
          policy.type === "autoRestart"
            ? "auto-restart policy"
            : "launch policy",
        );
      } else {
        failedParts.push(
          policy.type === "autoRestart"
            ? "auto-restart policy"
            : "launch policy",
        );
      }
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
  // Runtime: UpdatePersonaRequest is a full write — omitted/undefined runtime
  // means "clear this field to null". Compare against observed null when
  // submitted.runtime is undefined (clear semantics). If submitted.runtime is
  // a non-empty string, compare against that value.
  const submittedRuntime = submitted.runtime ?? null;
  if ((observed.runtime ?? null) !== submittedRuntime) return false;
  if ((observed.model ?? null) !== (submitted.model ?? null)) return false;
  if ((observed.provider ?? null) !== (submitted.provider ?? null))
    return false;
  if (!namePoolEqual(observed.namePool, submitted.namePool ?? [])) return false;
  if (!envVarsMapEqual(observed.envVars, submitted.envVars ?? {})) return false;
  // Behavior: a submitted group is a full-replacement unit (definition-only
  // context). The backend replaces respondTo/allowlist/parallelism as a set,
  // clearing any OMITTED member to null/empty — so settlement must compare
  // every member, including omitted ones, against the observed cleared value.
  // Skipping undefined members (the prior `!== undefined` guards) let a clear
  // the backend failed to apply false-succeed.
  if (submitted.behavior !== undefined) {
    const b = submitted.behavior;
    if ((observed.respondTo ?? null) !== (b.respondTo ?? null)) return false;
    // The backend stores the allowlist only in allowlist mode and clears it to
    // empty otherwise, so an omitted allowlist settles against an empty list.
    if (
      observed.respondToAllowlist.join(",") !==
      (b.respondToAllowlist ?? []).join(",")
    )
      return false;
    if ((observed.parallelism ?? null) !== (b.parallelism ?? null))
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
  // Harness-pin fields (Artifact 3 / Thufir pass-2 CRITICAL-3)
  if (submitted.agentCommand !== undefined) {
    if (submitted.agentCommand === "") {
      // "" is the sentinel for "inherit/unpin" — the backend clears
      // agentCommandOverride. Settle against agentCommandOverride === null,
      // NOT against agentCommand (which carries the resolved effective command).
      if ((observed.agentCommandOverride ?? null) !== null) return false;
    } else {
      // Explicit pin — compare the stored command.
      if ((submitted.agentCommand ?? "") !== (observed.agentCommand ?? ""))
        return false;
    }
  }
  if (submitted.harnessOverride !== undefined) {
    // harnessOverride=true means agentCommand is a pin (agentCommandOverride non-null).
    // harnessOverride=false means inherit (agentCommandOverride null).
    const submittedIsPinned = submitted.harnessOverride === true;
    const observedIsPinned = (observed.agentCommandOverride ?? null) !== null;
    if (submittedIsPinned !== observedIsPinned) return false;
  }
  if (
    submitted.agentArgs !== undefined &&
    submitted.agentArgs.join(",") !== (observed.agentArgs ?? []).join(",")
  ) {
    return false;
  }
  if (
    submitted.acpCommand !== undefined &&
    (submitted.acpCommand ?? "") !== (observed.acpCommand ?? "")
  ) {
    return false;
  }
  return true;
}

/**
 * Check whether a single policy setter reached the observed agent state.
 * Used for per-boundary and final settlement of L-field writes.
 */
function observedPolicyMatches(
  observed: ManagedAgent,
  policy:
    | { type: "autoRestart"; pubkey: string; value: boolean }
    | { type: "startOnAppLaunch"; pubkey: string; value: boolean },
): boolean {
  if (policy.type === "autoRestart") {
    return observed.autoRestartOnConfigChange === policy.value;
  }
  return observed.startOnAppLaunch === policy.value;
}

function capitalizeFirst(s: string): string {
  if (!s) return s;
  return s.charAt(0).toUpperCase() + s.slice(1);
}

// Re-export for convenient use in AgentEditDialog
export { seedAgentFormModel, emitAgentFormDiff } from "./agentFormModel";
export type { AgentFormModel, AgentEditContext } from "./agentFormModel";
