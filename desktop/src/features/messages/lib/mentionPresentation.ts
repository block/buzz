/** Presentation only. Publication still performs fresh authorization. */
export type MentionAction = "mention" | "invite" | "checking" | "unavailable";

export function isMentionActionable(candidate: { action?: MentionAction }) {
  return candidate.action !== "checking" && candidate.action !== "unavailable";
}
