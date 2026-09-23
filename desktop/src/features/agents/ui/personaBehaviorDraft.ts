import type {
  AcpSessionPolicy,
  PermissionPolicy,
  PersonaBehaviorInput,
  RespondToMode,
} from "@/shared/api/types";

/**
 * Dialog-side draft of a definition's NIP-AP behavioral group.
 *
 * `respondTo: null` means "unset" — the definition carries no mode and the
 * harness default (owner-only) applies at mint. The distinction matters for
 * wire bytes, not semantics: a definition without behavioral fields must
 * stay without behavioral fields
 * through unrelated edits so its published content (and content hash) does
 * not move.
 */
export type PersonaBehaviorDraft = {
  respondTo: RespondToMode | null;
  respondToAllowlist: string[];
  /** Raw text; only `parseInt > 0` submits (legacy dialog parity). */
  parallelism: string;
  /**
   * Definition-level default permission policy — resolver tier 2. `null`
   * means "no default" (defer to global/built-in). Local-only: it is never
   * published, so it does not affect the definition's content hash.
   */
  permissionPolicy: PermissionPolicy | null;
  sessionPolicy: AcpSessionPolicy;
};

export const emptyPersonaBehaviorDraft: PersonaBehaviorDraft = {
  respondTo: null,
  respondToAllowlist: [],
  parallelism: "",
  permissionPolicy: null,
  sessionPolicy: "channel",
};

/** Seed the draft from a dialog-state behavior group (edit/duplicate). */
export function draftFromBehavior(
  behavior: PersonaBehaviorInput | undefined,
): PersonaBehaviorDraft {
  return {
    respondTo: behavior?.respondTo ?? null,
    respondToAllowlist: [...(behavior?.respondToAllowlist ?? [])],
    parallelism:
      behavior?.parallelism != null ? String(behavior.parallelism) : "",
    permissionPolicy: behavior?.permissionPolicy ?? null,
    sessionPolicy: behavior?.sessionPolicy ?? "channel",
  };
}

/**
 * Allowlist-mode crash-loop guard (re-homed from the legacy create dialog):
 * an empty allowlist would crash every instance minted from the definition
 * at startup, so submit is blocked in create AND edit mode. The server-side
 * chokepoint (`apply_persona_behavior`) enforces the same rule.
 */
export function personaBehaviorDraftValid(draft: PersonaBehaviorDraft) {
  return draft.respondTo !== "allowlist" || draft.respondToAllowlist.length > 0;
}

function behaviorFromDraft(
  draft: PersonaBehaviorDraft,
): PersonaBehaviorInput | undefined {
  const parallelism = Number.parseInt(draft.parallelism, 10);
  const respondTo = draft.respondTo ?? undefined;
  const resolvedParallelism = parallelism > 0 ? parallelism : undefined;
  const permissionPolicy = draft.permissionPolicy ?? undefined;
  const { sessionPolicy } = draft;

  // Session scope has content when respondTo, parallelism, or a non-channel
  // sessionPolicy is present. Permission scope has content when permissionPolicy
  // is set. A completely empty draft (nothing in either scope) returns undefined
  // so callers can distinguish "no-op" from "submit the channel default".
  const hasSessionFields =
    respondTo !== undefined ||
    resolvedParallelism !== undefined ||
    sessionPolicy !== "channel";
  const hasPermissionField = permissionPolicy !== undefined;

  if (!hasSessionFields && !hasPermissionField) {
    return undefined;
  }

  return {
    respondTo,
    // Mode and list travel as a unit; a list without allowlist mode is
    // stale data the author didn't choose (legacy dialog parity).
    respondToAllowlist:
      respondTo === "allowlist" ? draft.respondToAllowlist : undefined,
    parallelism: resolvedParallelism,
    // Include permissionPolicy only when non-null; omitting it means "clear"
    // within a present behavior group (replace-as-a-unit semantics).
    ...(hasPermissionField && { permissionPolicy }),
    // Include sessionPolicy only when the session scope has something to say.
    // A permission-policy-only group omits it; the backend treats a missing
    // sessionPolicy as the channel default — it does NOT preserve the old value.
    ...(hasSessionFields && { sessionPolicy }),
  };
}

/**
 * Resolve the behavior group a persona submit should carry.
 *
 * Absent (`undefined`) means "don't touch the stored behavior group"
 * server-side, so:
 * - a behavior group that is untouched relative to its seed submits nothing — an
 *   unrelated edit (rename, prompt tweak) must not rewrite the published
 *   definition's behavior bytes or flip its content hash;
 * - creates always submit the selected session policy; the channel default is
 *   omitted from durable/public JSON by the backend for wire compatibility;
 * - any real change submits the full group (replace-as-a-unit semantics);
 * - clearing optional fields on edit still submits an explicit group so the
 *   server-side clear is not silently no-oped.
 *
 * Duplicate flows pass the source persona's behavior group as `seed` but with
 * `isEdit: false`: a duplicate is a CREATE, so a non-empty inherited
 * behavior group
 * must be submitted even though it equals the seed.
 */
export function behaviorForSubmit(
  draft: PersonaBehaviorDraft,
  seed: PersonaBehaviorDraft,
  isEdit: boolean,
): PersonaBehaviorInput | undefined {
  const group = behaviorFromDraft(draft);
  if (!isEdit) {
    // Creates always submit the selected session policy so the backend can
    // record the channel default explicitly; it omits the channel default from
    // durable/public JSON for wire compatibility.
    return (
      group ?? {
        respondTo: undefined,
        respondToAllowlist: undefined,
        parallelism: undefined,
        sessionPolicy: draft.sessionPolicy,
      }
    );
  }
  const seedGroup = behaviorFromDraft(seed);
  if (JSON.stringify(group) === JSON.stringify(seedGroup)) {
    return undefined;
  }
  if (group !== undefined) {
    return group;
  }
  // Draft is completely empty (nothing in either scope). Check whether the
  // seed had session-scoped fields: if so, submit the channel default from the
  // draft to explicitly clear them server-side. If the seed had only a
  // permission policy, submit {} to clear just that field.
  // NOTE: use draft.sessionPolicy (the cleared default), not seed.sessionPolicy,
  // so that editing "Each thread → Entire channel" with no other fields set
  // actually submits "channel" rather than silently resubmitting "thread".
  const seedHadSessionFields =
    seed.respondTo !== null ||
    Number.parseInt(seed.parallelism, 10) > 0 ||
    seed.sessionPolicy !== "channel";
  return seedHadSessionFields
    ? {
        respondTo: undefined,
        respondToAllowlist: undefined,
        parallelism: undefined,
        sessionPolicy: draft.sessionPolicy,
      }
    : {};
}
