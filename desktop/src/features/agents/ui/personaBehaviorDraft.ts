import type { PersonaBehaviorInput, RespondToMode } from "@/shared/api/types";

/**
 * Dialog-side draft of a definition's NIP-AP behavioral quad.
 *
 * `respondTo: null` means "unset" — the definition carries no mode and the
 * harness default (owner-only) applies at mint. The distinction matters for
 * wire bytes, not semantics: a quad-less definition must stay quad-less
 * through unrelated edits so its published content (and content hash) does
 * not move.
 */
export type PersonaBehaviorDraft = {
  respondTo: RespondToMode | null;
  respondToAllowlist: string[];
  /** Raw text; trimmed-empty means unset. */
  mcpToolsets: string;
  /** Raw text; only `parseInt > 0` submits (legacy dialog parity). */
  parallelism: string;
};

export const emptyPersonaBehaviorDraft: PersonaBehaviorDraft = {
  respondTo: null,
  respondToAllowlist: [],
  mcpToolsets: "",
  parallelism: "",
};

/** Seed the draft from a dialog-state behavior group (edit/duplicate). */
export function draftFromBehavior(
  behavior: PersonaBehaviorInput | undefined,
): PersonaBehaviorDraft {
  return {
    respondTo: behavior?.respondTo ?? null,
    respondToAllowlist: [...(behavior?.respondToAllowlist ?? [])],
    mcpToolsets: behavior?.mcpToolsets ?? "",
    parallelism:
      behavior?.parallelism != null ? String(behavior.parallelism) : "",
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
    mcpToolsets: draft.mcpToolsets.trim() || undefined,
    parallelism: parallelism > 0 ? parallelism : undefined,
  };
  const isEmpty =
    group.respondTo === undefined &&
    group.mcpToolsets === undefined &&
    group.parallelism === undefined;
  return isEmpty ? undefined : group;
}

/**
 * Resolve the behavior group a persona submit should carry.
 *
 * Absent (`undefined`) means "don't touch the stored quad" server-side, so:
 * - a quad that is untouched relative to its seed submits nothing — an
 *   unrelated edit (rename, prompt tweak) must not rewrite the published
 *   definition's quad bytes or flip its content hash;
 * - an empty quad submits nothing — plain creates stay quad-less;
 * - any real change submits the full group (replace-all-four semantics).
 *
 * Duplicate flows pass the source persona's quad as `seed` but with
 * `isEdit: false`: a duplicate is a CREATE, so a non-empty inherited quad
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
  return JSON.stringify(group) === JSON.stringify(seedGroup)
    ? undefined
    : group;
}
