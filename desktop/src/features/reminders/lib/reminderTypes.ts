export type ReminderStatus = "pending" | "done" | "cancelled";

export type ReminderTarget = {
  /** Event ID of the message being reminded about. */
  eventId: string;
  /** Channel ID where the message lives. */
  channelId: string;
  /** Preview text of the target message (truncated). */
  preview: string;
  /** Author pubkey of the target message. */
  authorPubkey: string;
};

export type ReminderContent = {
  /** Target message. Absent for note-only reminders (NIP-ER allows either). */
  target?: ReminderTarget;
  /** Optional user-provided note. */
  note?: string;
  status: ReminderStatus;
};

export type Reminder = {
  /** The d-tag (unique identifier for this reminder). */
  id: string;
  /** Unix timestamp (seconds) when the reminder is due. Absent for done/cancelled. */
  notBefore?: number;
  /** Decrypted reminder content. */
  content: ReminderContent;
  /** Event created_at timestamp. */
  createdAt: number;
  /** The raw event ID on the relay. */
  eventId: string;
};
