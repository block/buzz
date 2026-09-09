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

/**
 * Marker the backend prefixes onto a compare-and-swap rejection (see
 * `commands/personas/update.rs::PERSONA_REVISION_CONFLICT`). A thrown error
 * containing it means the persisted definition advanced past the revision this
 * editor was seeded with — the coordinator maps it to the "changed while you
 * were editing" affordance instead of a generic save failure.
 */
export const PERSONA_REVISION_CONFLICT = "persona-revision-conflict:";

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
  /**
   * The definition `updatedAt` observed when the form was seeded. When a
   * `personaInput` is submitted, the coordinator compares this against the
   * latest `ctx` definition and aborts before any write if they differ — the
   * definition was revised by another writer while this form was open, so the
   * stale form would clobber the newer values. Undefined/null skips the check
   * (e.g. instance-only saves, which emit no `personaInput`).
   */
  expectedDefinitionUpdatedAt?: string | null;
  /** Validated runtime catalog for the runtime-edit gate. */
  runtimes?: readonly AcpRuntimeCatalogEntry[];

  // Mutations
  updatePersona: (input: UpdatePersonaInput) => Promise<unknown>;
  updatePersonaAndPublish: (
    input: UpdatePersonaInput,
  ) => Promise<PersonaSharePublicationResult>;
  /**
   * Publish the current on-disk definition to the catalog without writing any
   * field changes. Called when the initial save+publish succeeded on disk but
   * the publication step threw — the persona is persisted, so the retry only
   * needs to re-attempt the relay submission.  Uses the share-toggle strict
   * path (`set_persona_shared`) which reads the definition from disk, so no
   * field payload is needed.  If omitted the coordinator falls back to a
   * terminal "saved locally, not published" report.
   */
  publishRetry?: (personaId: string) => Promise<PersonaSharePublicationResult>;
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
    expectedDefinitionUpdatedAt,
    runtimes,
    updatePersona,
    updatePersonaAndPublish,
    updateManagedAgent,
    setAutoRestart,
    setStartOnAppLaunch,
    publishRetry,
    refetchStores,
    onDone,
    onSavedWhileStopped,
  } = opts;

  const def = editContextDefinition(ctx);
  const inst = editContextInstance(ctx);

  // Settlement refetch that never lets a rejection escape the save path. A
  // rejection means we could not OBSERVE whether the preceding write persisted
  // — a "verification unknown" state distinct from an observed non-persist. The
  // write may well have committed, so callers must stop advancing and report
  // that persistence is unverified rather than claiming the write failed.
  const verifiedRefetch = async (): Promise<
    | {
        verified: true;
        persona: AgentPersona | null;
        agent: ManagedAgent | null;
      }
    | { verified: false }
  > => {
    try {
      const { persona, agent } = await refetchStores();
      return { verified: true, persona, agent };
    } catch {
      return { verified: false };
    }
  };

  // Bail-out for a refetch rejection: dialog stays open (return false) and the
  // toast states persistence is unverified — never that the write failed.
  const reportVerificationUnknown = (): false => {
    const name = def?.displayName ?? inst?.name ?? "the agent";
    toast.warning(
      `Could not verify whether ${name}'s changes saved — they may have been applied. Reopen the editor to check before retrying.`,
    );
    return false;
  };

  // ── Step 0: Validate ──────────────────────────────────────────────────────
  // Concurrent-edit guard: a submitted definition write is built from the form
  // baseline captured at seed time. This pre-write refetch is a cheap early
  // exit — it reads the persisted definition and aborts before any write when
  // another writer has ALREADY committed a newer revision, which is the common
  // stale-dialog case. It does NOT by itself close the check-to-write window:
  // the authoritative read below releases the store lock before the write
  // reacquires it, so a writer that commits in that interval would slip
  // through. The backend `update_persona`/`update_persona_and_publish` carry
  // `expectedUpdatedAt` and re-compare it under the SAME lock that guards the
  // write (a compare-and-swap), which is what actually closes the window; a
  // rejection there surfaces as the same "changed while you were editing"
  // toast (see Step 1).
  //
  // The cached `ctx.updatedAt` is NOT authoritative: writer B can submit before
  // its React-query cache receives writer A's newer write, so both the cache
  // and the seed still read the pre-A revision. A cache-only comparison passes
  // (stale-equal) and B would reach the write. The forced refetch reads
  // persisted state, catching an already-committed A before any write.
  if (personaInput && def && expectedDefinitionUpdatedAt != null) {
    const preCheck = await verifiedRefetch();
    if (!preCheck.verified) {
      // Refetch rejected BEFORE any write — nothing was attempted, so this is a
      // pre-save verification failure, not an unknown-persistence state.
      toast.error(
        `Could not verify the latest version of ${def.displayName} before saving — nothing was changed. Try again.`,
      );
      return false;
    }
    if ((preCheck.persona?.updatedAt ?? null) !== expectedDefinitionUpdatedAt) {
      toast.error(
        `${def.displayName} changed while you were editing — reopen the editor to get the latest before saving.`,
      );
      return false;
    }
  }

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
  // True when the persona fields persisted but the catalog publication step
  // threw before it could return a status (publish path only). The profile is
  // on disk, but the relay was never reached or enqueued, so settlement must
  // not close as full success — it must report "saved but not published".
  let publishFailed = false;

  // Step 1: Definition write — settle immediately, stop if not persisted.
  if (personaInput) {
    // Carry the seed-time revision into the write so the backend can reject a
    // concurrent overwrite under its store lock (compare-and-swap). This is the
    // authoritative close of the check-to-write window; the Step-0 refetch only
    // catches an already-committed writer.
    const guardedInput: UpdatePersonaInput = {
      ...personaInput,
      expectedUpdatedAt: expectedDefinitionUpdatedAt ?? undefined,
    };
    let caughtError: string | null = null;
    try {
      if (publishCatalogUpdates) {
        const result = await updatePersonaAndPublish(guardedInput);
        publicationStatus = result.publicationStatus;
      } else {
        await updatePersona(guardedInput);
      }
    } catch (err) {
      const message =
        err instanceof Error ? err.message : "Failed to save agent profile.";
      // A backend compare-and-swap rejection means the definition advanced
      // while this editor was open — the same user situation as the Step-0
      // drift abort. Report it as such and bail without settling: nothing was
      // written, so there is no persistence to verify.
      if (message.startsWith(PERSONA_REVISION_CONFLICT)) {
        toast.error(
          `${def?.displayName ?? "This agent"} changed while you were editing — reopen the editor to get the latest before saving.`,
        );
        return false;
      }
      caughtError = message;
    }
    // Settle after definition write — re-fetch regardless of throw. Observed
    // persistence (not the command result) decides whether the step failed: a
    // throw whose write is on disk is NOT a failed step, and a silent no-op
    // that did not persist IS. Only a genuine non-persisted write stops advance.
    const settle = await verifiedRefetch();
    if (!settle.verified) return reportVerificationUnknown();
    const settled = settle.persona;
    const persisted =
      settled !== null &&
      observedStateMatchesPersonaInput(settled, personaInput);
    if (!persisted) {
      firstError =
        caughtError ?? "Agent profile did not persist — reopen to retry.";
    } else if (publishCatalogUpdates && caughtError !== null) {
      // The persona fields made it to disk (persisted = true) but the publish
      // command threw before it could return a status — the catalog was never
      // reached. Mark this so final settlement does not close as full success:
      // "save and publish" promised a relay outcome that was not delivered.
      publishFailed = true;
      // For combined D+I or D+L saves, settle publication independently now —
      // before advancing to I/L writes — so a transient preparation failure
      // does not permanently bypass recovery. If retry succeeds, clear
      // publishFailed/firstError and proceed normally. If retry fails (or no
      // seam), set firstError to block I/L advancement: the partial-failure
      // toast will name both the unsaved I/L remainder and the unpublished
      // catalog, since a fresh reopen seeds persisted values with no
      // personaInput, meaning publication is never re-attempted.
      const hasPendingIL = agentInput !== null || policySets.length > 0;
      if (hasPendingIL) {
        // Combined D+I or D+L save: settle publication before advancing to I/L.
        if (publishRetry && def) {
          let earlyRetryError: string | null = null;
          try {
            const earlyRetryResult = await publishRetry(def.id);
            publicationStatus = earlyRetryResult.publicationStatus;
            // Retry succeeded — publication settled. Clear both flags so I/L
            // writes can continue and the final success branch fires.
            publishFailed = false;
            firstError = null;
          } catch (err) {
            earlyRetryError =
              err instanceof Error ? err.message : "unknown error";
          }
          if (publishFailed) {
            // Retry also failed — block I/L advancement and carry the reason
            // as firstError. The partial-failure toast will name the profile as
            // saved and the I/L remainder as not saved, with the catalog error
            // as the failure reason — reopen/retry is accurate for the I/L
            // remainder; the D change IS kept, catalog publication is not.
            firstError = `catalog publication failed: ${earlyRetryError ?? caughtError ?? "unknown error"}`;
          }
        } else {
          // No retry seam — block I/L advancement for the same reason.
          firstError = `catalog publication failed: ${caughtError ?? "unknown error"}`;
        }
      } else {
        // Definition-only save: the final `!observedRemainder && publishFailed`
        // block handles publication retry after all settlement.
        firstError = caughtError;
      }
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
    const settle = await verifiedRefetch();
    if (!settle.verified) return reportVerificationUnknown();
    const settled = settle.agent;
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
      const settle = await verifiedRefetch();
      if (!settle.verified) return reportVerificationUnknown();
      const settled = settle.agent;
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
  const finalSettle = await verifiedRefetch();
  if (!finalSettle.verified) return reportVerificationUnknown();
  const observedPersona = finalSettle.persona;
  const observedAgent = finalSettle.agent;

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

  if (!observedRemainder && publishFailed) {
    // The persona persisted but the publish command threw before returning a
    // status — the relay was never reached. Attempt a publish-only retry via
    // set_persona_shared (reads the current on-disk definition; no field write
    // needed). If the retry succeeds, fall through to full success. If it
    // fails (or no retry seam is available), report only that the profile saved
    // locally and could not be published — no automatic-retry promise and no
    // reopen instruction (a fresh reopen seeds persisted values, emits null
    // personaInput, and the coordinator never re-attempts publication).
    const personaName =
      observedPersona?.displayName ??
      personaInput?.displayName ??
      def?.displayName ??
      "Agent";
    if (publishRetry && def) {
      let retryError: string | null = null;
      try {
        const retryResult = await publishRetry(def.id);
        publicationStatus = retryResult.publicationStatus;
        // Retry succeeded — the persona is published. Clear publishFailed and
        // fall through to the full-success branch below.
        publishFailed = false;
      } catch (err) {
        retryError = err instanceof Error ? err.message : "unknown error";
      }
      if (publishFailed) {
        // Retry also failed — terminal state: the preparation step threw, so
        // no durable pending row was created. Report only what is true: the
        // persona is saved locally but the catalog was not reached.
        toast.warning(
          `${personaName} saved locally, but could not be published to the catalog: ${retryError ?? firstError ?? "unknown error"}.`,
        );
        return false;
      }
      // publishFailed is now false — fall through to full success.
    } else {
      // No retry seam available — terminal state: same honest shape.
      toast.warning(
        `${personaName} saved locally, but could not be published to the catalog: ${firstError ?? "unknown error"}.`,
      );
      return false;
    }
  }

  if (!observedRemainder) {
    // Full success — every submitted write is reflected in observed state.
    // Prefer the OBSERVED name from the final refetch over `latestAgent`.
    // `latestAgent` only advances on a non-throwing `updateManagedAgent`; a
    // rename that persisted but whose command threw after commit leaves it at
    // the pre-save `inst`, so a committed Alice→Bob would report "Alice saved."
    // The observed refetch reflects what is actually on disk (the stopped-state
    // path below already prefers `observedAgent` for the same reason).
    const agentName =
      observedAgent?.name ?? latestAgent?.name ?? def?.displayName ?? "Agent";

    if (profileSyncError) {
      showAgentProfileSyncWarning(agentName, profileSyncError);
    } else if (publishCatalogUpdates && personaInput) {
      // The publish notice reports the DEFINITION update, so name it from the
      // observed persona (disk state) — not `agentName`, which is
      // instance-oriented and falls back to the pre-save `def.displayName` on a
      // definition-only edit where both agent candidates are null. Fall back to
      // the submitted rename, then the pre-save snapshot.
      const personaName =
        observedPersona?.displayName ??
        personaInput.displayName ??
        def?.displayName ??
        "Agent";
      toast.success(personaSaveNotice(personaName, publicationStatus));
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
  // Description: full-write semantics (the dialog always sends the current
  // value; absent/empty clears). Mirror normalize_description: trim, then
  // blank/absent → null. Do NOT strip prohibited bytes — a U+200B description
  // must NOT appear to match an unchanged or cleared observed value, because
  // the Rust backend rejected it before writing.
  const submittedDescRaw = submitted.description ?? null;
  const submittedDesc =
    submittedDescRaw !== null ? submittedDescRaw.trim() || null : null;
  if ((observed.description ?? null) !== submittedDesc) return false;
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
  // Effort level — tri-state:
  //   absent submission (undefined) → skip (field not being written)
  //   null submission               → clear; settled when observed column is null/absent
  //   string submission             → set; settled when observed column equals submitted
  // Mirrors the backend's canonical storage semantics (no byte stripping here —
  // a prohibited-byte value must NOT launder into apparent success).
  if (submitted.effortLevel !== undefined) {
    const submittedEffort = submitted.effortLevel ?? null;
    const observedEffort = observed.effortLevel ?? null;
    if (submittedEffort !== observedEffort) return false;
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
