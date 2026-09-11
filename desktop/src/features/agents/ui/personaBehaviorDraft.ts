import type {
  AcpSessionPolicy,
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
  sessionPolicy: AcpSessionPolicy;
};

export const emptyPersonaBehaviorDraft: PersonaBehaviorDraft = {
  respondTo: null,
  respondToAllowlist: [],
  parallelism: "",
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
  const group: PersonaBehaviorInput = {
    respondTo: draft.respondTo ?? undefined,
    // Mode and list travel as a unit; a list without allowlist mode is
    // stale data the author didn't choose (legacy dialog parity).
    respondToAllowlist:
      draft.respondTo === "allowlist" ? draft.respondToAllowlist : undefined,
    parallelism: parallelism > 0 ? parallelism : undefined,
    sessionPolicy: draft.sessionPolicy,
  };
  return group;
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
 * - clearing the optional fields on edit still submits the channel default,
 *   because "submit nothing" would silently no-op the clear.
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
    return group;
  }
  const seedGroup = behaviorFromDraft(seed);
  if (JSON.stringify(group) === JSON.stringify(seedGroup)) {
    return undefined;
  }
  return group ?? {};
}
