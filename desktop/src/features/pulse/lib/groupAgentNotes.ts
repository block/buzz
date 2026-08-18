import type { UserNote } from "@/shared/api/socialTypes";

/** A group of consecutive notes from the same agent within a time window. */
export type AgentNoteGroup = {
  /** Agent pubkey. */
  pubkey: string;
  /** All notes in this group, newest first. */
  notes: UserNote[];
  /** Timestamp of the most recent note in the group. */
  latestAt: number;
  /** Timestamp of the oldest note in the group. */
  earliestAt: number;
};

/**
 * Groups notes by agent pubkey + time proximity.
 * Consecutive notes from the same agent are collapsed into a single group
 * as long as the gap between adjacent notes is within `windowSeconds`.
 * Input notes must be sorted newest-first.
 */
export function groupAgentNotes(
  notes: UserNote[],
  windowSeconds = 300,
): AgentNoteGroup[] {
  if (notes.length === 0) return [];

  const groups: AgentNoteGroup[] = [];
  let current: AgentNoteGroup | null = null;

  for (const note of notes) {
    // Compare against the previous (most recently added) note in the group,
    // not the group's earliest — prevents the window from growing unboundedly.
    const lastNoteInGroup = current?.notes[current.notes.length - 1];
    if (
      current &&
      lastNoteInGroup &&
      current.pubkey === note.pubkey &&
      lastNoteInGroup.createdAt - note.createdAt <= windowSeconds
    ) {
      current.notes.push(note);
      current.earliestAt = note.createdAt;
    } else {
      if (current) groups.push(current);
      current = {
        pubkey: note.pubkey,
        notes: [note],
        latestAt: note.createdAt,
        earliestAt: note.createdAt,
      };
    }
  }

  if (current) groups.push(current);
  return groups;
}
