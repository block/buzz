import { nip19 } from "nostr-tools";

import type { UserNote } from "@/shared/api/socialTypes";

export function buildNoteShareUri(note: Pick<UserNote, "id" | "pubkey">) {
  return `nostr:${nip19.neventEncode({
    id: note.id,
    author: note.pubkey,
  })}`;
}

export function toggleNoteIdInSet(
  current: ReadonlySet<string>,
  noteId: string,
  enabled: boolean,
) {
  const next = new Set(current);
  if (enabled) {
    next.add(noteId);
  } else {
    next.delete(noteId);
  }
  return next;
}

export function applyReactionState(
  current:
    | Map<string, { count: number; reactedByCurrentUser: boolean }>
    | undefined,
  noteId: string,
  reactedByCurrentUser: boolean,
) {
  const next = new Map(current);
  const previous = next.get(noteId) ?? {
    count: 0,
    reactedByCurrentUser: false,
  };
  const count = Math.max(
    0,
    previous.count +
      (reactedByCurrentUser && !previous.reactedByCurrentUser ? 1 : 0) -
      (!reactedByCurrentUser && previous.reactedByCurrentUser ? 1 : 0),
  );
  next.set(noteId, {
    count,
    reactedByCurrentUser,
  });
  return next;
}

export function isDuplicateReactionError(error: unknown) {
  return (
    error instanceof Error &&
    error.message.toLowerCase().includes("duplicate: reaction already exists")
  );
}
