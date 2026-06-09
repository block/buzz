export type UserNote = {
  id: string;
  pubkey: string;
  createdAt: number;
  content: string;
  tags: string[][];
};

export type NoteReactionSummary = {
  noteId: string;
  emoji: string;
  count: number;
  pubkeys: string[];
};

export type UserNotesCursor = {
  before: number;
  beforeId: string;
};

export type UserNotesResponse = {
  notes: UserNote[];
  nextCursor: UserNotesCursor | null;
};

export type PublishNoteResult = {
  eventId: string;
  accepted: boolean;
  message: string;
};

export type ContactEntry = {
  pubkey: string;
  relayUrl?: string;
  petname?: string;
};

export type ContactListResponse = {
  id: string;
  pubkey: string;
  createdAt: number;
  contacts: ContactEntry[];
};
