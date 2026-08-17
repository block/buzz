import type {
  ContactListResponse,
  NoteReactionSummary,
  PublishNoteResult,
  UserNote,
  UserNotesResponse,
} from "@/shared/api/socialTypes";

import { invokeTauri } from "./tauri";

type RawUserNote = {
  id: string;
  pubkey: string;
  created_at: number;
  content: string;
  tags: string[][];
};

type RawNoteReactionSummary = {
  note_id: string;
  emoji: string;
  count: number;
  pubkeys: string[];
};

type RawUserNotesCursor = {
  before: number;
  before_id: string;
};

type RawUserNotesResponse = {
  notes: RawUserNote[];
  next_cursor: RawUserNotesCursor | null;
};

type RawContactListResponse = {
  id: string;
  pubkey: string;
  created_at: number;
  tags: string[][];
  content: string;
};

function fromRawUserNote(note: RawUserNote): UserNote {
  return {
    id: note.id,
    pubkey: note.pubkey,
    createdAt: note.created_at,
    content: note.content,
    tags: note.tags,
  };
}

export async function getNoteReactions(
  noteIds: string[],
): Promise<NoteReactionSummary[]> {
  const response = await invokeTauri<RawNoteReactionSummary[]>(
    "get_note_reactions",
    { noteIds },
  );
  return response.map((summary) => ({
    noteId: summary.note_id,
    emoji: summary.emoji,
    count: summary.count,
    pubkeys: summary.pubkeys,
  }));
}

export async function getNote(noteId: string): Promise<UserNote | null> {
  const response = await invokeTauri<RawUserNote | null>("get_note", {
    noteId,
  });
  return response ? fromRawUserNote(response) : null;
}

export async function getUserNotes(
  pubkey: string,
  options?: {
    limit?: number;
    before?: number;
    beforeId?: string;
  },
): Promise<UserNotesResponse> {
  const response = await invokeTauri<RawUserNotesResponse>("get_user_notes", {
    pubkey,
    limit: options?.limit ?? null,
    before: options?.before ?? null,
    beforeId: options?.beforeId ?? null,
  });

  return {
    notes: response.notes.map(fromRawUserNote),
    nextCursor: response.next_cursor
      ? {
          before: response.next_cursor.before,
          beforeId: response.next_cursor.before_id,
        }
      : null,
  };
}

type RawPublishNoteResult = {
  event_id: string;
  accepted: boolean;
  message: string;
};

export async function publishNote(
  content: string,
  replyTo?: string,
  mentionPubkeys?: string[],
  mediaTags?: string[][],
): Promise<PublishNoteResult> {
  const raw = await invokeTauri<RawPublishNoteResult>("publish_note", {
    content,
    replyTo: replyTo ?? null,
    mentionPubkeys: mentionPubkeys ?? null,
    mediaTags: mediaTags ?? null,
  });
  return {
    eventId: raw.event_id,
    accepted: raw.accepted,
    message: raw.message,
  };
}

export async function getContactList(
  pubkey: string,
): Promise<ContactListResponse> {
  const raw = await invokeTauri<RawContactListResponse>("get_contact_list", {
    pubkey,
  });

  const contacts = raw.tags
    .filter((t) => t[0] === "p" && t[1])
    .map((t) => ({
      pubkey: t[1],
      relayUrl: t[2] || undefined,
      petname: t[3] || undefined,
    }));

  return {
    id: raw.id,
    pubkey: raw.pubkey,
    createdAt: raw.created_at,
    contacts,
  };
}

export async function setContactList(
  contacts: Array<{
    pubkey: string;
    relayUrl?: string;
    petname?: string;
  }>,
): Promise<PublishNoteResult> {
  const raw = await invokeTauri<RawPublishNoteResult>("set_contact_list", {
    contacts: contacts.map((c) => ({
      pubkey: c.pubkey,
      relay_url: c.relayUrl ?? null,
      petname: c.petname ?? null,
    })),
  });
  return {
    eventId: raw.event_id,
    accepted: raw.accepted,
    message: raw.message,
  };
}

export async function getLikedNotes(
  authorPubkey: string,
  limit?: number,
): Promise<UserNotesResponse> {
  const response = await invokeTauri<RawUserNotesResponse>("get_liked_notes", {
    authorPubkey,
    limit: limit ?? null,
  });

  return {
    notes: response.notes.map(fromRawUserNote),
    nextCursor: response.next_cursor
      ? {
          before: response.next_cursor.before,
          beforeId: response.next_cursor.before_id,
        }
      : null,
  };
}

export async function getGlobalNotes(options?: {
  limit?: number;
  before?: number;
  beforeId?: string;
}): Promise<UserNotesResponse> {
  const response = await invokeTauri<RawUserNotesResponse>("get_global_notes", {
    limit: options?.limit ?? null,
    before: options?.before ?? null,
    beforeId: options?.beforeId ?? null,
  });

  return {
    notes: response.notes.map(fromRawUserNote),
    nextCursor: response.next_cursor
      ? {
          before: response.next_cursor.before,
          beforeId: response.next_cursor.before_id,
        }
      : null,
  };
}

export async function getNotesTimeline(
  pubkeys: string[],
  limitPerUser?: number,
): Promise<UserNotesResponse> {
  const response = await invokeTauri<RawUserNotesResponse>(
    "get_notes_timeline",
    {
      pubkeys,
      limitPerUser: limitPerUser ?? null,
    },
  );

  return {
    notes: response.notes.map(fromRawUserNote),
    nextCursor: response.next_cursor
      ? {
          before: response.next_cursor.before,
          beforeId: response.next_cursor.before_id,
        }
      : null,
  };
}
