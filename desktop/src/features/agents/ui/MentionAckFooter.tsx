/**
 * NIP-MR: the inline notice shown under a message whose agent mention went
 * nowhere.
 *
 * Rendered through `MessageTimeline`'s `messageFooters` slot rather than as a
 * `MessageRow` prop, which keeps it clear of that component's hand-written memo
 * comparator (AGENTS.md gotcha 7).
 */

import { useMentionProblem } from "@/features/agents/pendingMentionAckStore";

/**
 * Human-readable text for a decline reason slug.
 *
 * The point of each string is to name the next action. "Not accepting messages
 * from you" tells the sender to ask the agent's owner to widen `respond_to`,
 * which is otherwise entirely invisible — the harness default is owner-only and
 * the drop happens with nothing published in any direction.
 */
function describeReason(reason: string | null): string {
  switch (reason) {
    case "sender-not-allowed":
      return "is not accepting messages from you — its owner controls who can prompt it";
    case "no-matching-rule":
      return "is not configured to respond to messages like this one";
    case "busy":
      return "was already working on something and dropped this message";
    default:
      return "declined this mention";
  }
}

export type MentionAckFooterProps = {
  eventId: string;
  /** Resolves a pubkey to a display name; falls back to a short hex. */
  resolveName: (pubkey: string) => string;
};

export function MentionAckFooter({
  eventId,
  resolveName,
}: MentionAckFooterProps) {
  const problem = useMentionProblem(eventId);
  if (!problem) return null;

  const lines: string[] = [];
  for (const { pubkey, reason } of problem.declined) {
    lines.push(`${resolveName(pubkey)} ${describeReason(reason)}.`);
  }
  if (problem.silent.length > 0) {
    const names = problem.silent.map(resolveName).join(", ");
    lines.push(
      problem.silent.length === 1
        ? `${names} never picked this up — it may be offline.`
        : `${names} never picked this up — they may be offline.`,
    );
  }
  if (lines.length === 0) return null;

  return (
    <p
      className="mt-1.5 text-2xs text-muted-foreground"
      data-testid="mention-ack-footer"
    >
      {lines.join(" ")}
    </p>
  );
}
