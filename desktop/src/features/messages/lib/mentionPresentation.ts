import type { MentionCandidate } from "./mentionCandidates";
/** Presentation only. Publication still performs fresh authorization. */
export type MentionAction = "mention" | "invite" | "checking" | "unavailable";

export function isMentionActionable(candidate: { action?: MentionAction }) {
  return candidate.action !== "checking" && candidate.action !== "unavailable";
}

/** Mark collisions before filtering or capping the rendered result list. */
export function markMentionCollisions(candidates: MentionCandidate[]) {
  const keysByName = new Map<string, Set<string>>();
  for (const candidate of candidates) {
    const name = candidate.displayName?.trim().toLowerCase();
    if (!name) continue;
    const keys = keysByName.get(name) ?? new Set<string>();
    keys.add(
      candidate.pubkey ?? candidate.personaId ?? candidate.teamId ?? name,
    );
    keysByName.set(name, keys);
  }
  return candidates.map((candidate) => ({
    ...candidate,
    hasNameCollision:
      candidate.hasNameCollision ||
      (keysByName.get(candidate.displayName?.trim().toLowerCase() ?? "")
        ?.size ?? 0) > 1,
  }));
}
