/** Kind of Bestie list item shown in the Bestie DM RHS. */
export type BestieListKind = "todo" | "reminder";

/** Reminder recurrence. Null / omitted = one-off. */
export type BestieReminderRepeat =
  | { kind: "daily" }
  | { kind: "weekly"; weekday: number };

export type BestieListItem = {
  createdAt: number;
  /**
   * Local calendar day YYYY-MM-DD for todo grouping (drag target).
   * Reminders ignore this; null todos default to created-day at parse time.
   */
  dayKey: string | null;
  /** Unix seconds; reminders only. */
  dueAt: number | null;
  id: string;
  kind: BestieListKind;
  /** Reminders only — null means one-off. */
  repeat: BestieReminderRepeat | null;
  /** Lower sorts first within a group; starred block uses its own order. */
  sortOrder: number;
  /** Message id that created this item (agent-add path), when known. */
  sourceMessageId: string | null;
  /** Todos only — starred rows pin above date groups. */
  starred: boolean;
  status: "open" | "done";
  text: string;
  updatedAt: number;
};


/** Pending bare-clock reminder waiting for AM/PM confirm (client-resolved dueAt). */
export type BestiePendingReminderConfirm = {
  bareClock: {
    dayHint: "today" | "tomorrow" | null;
    hour: number;
    minute: number;
  };
  /** Unix ms when the confirm was opened. */
  createdAt: number;
  sourceMessageId: string;
  text: string;
};

export type BestieListState = {
  items: BestieListItem[];
  /** Bare clock awaiting AM/PM; cleared when reminder is created. */
  pendingReminderConfirm: BestiePendingReminderConfirm | null;
  /** Message ids already applied so reloads don't duplicate agent-add actions. */
  processedMessageIds: string[];
  version: 1;
};

/**
 * Runtime + persistence scope. Persistence keys use owner+relay only so
 * reminders/todos/jobs/coffee/scratch/threads survive agent reassignment.
 * `agentPubkey` remains for DM/wake/runtime callers.
 */
export type BestieListScope = {
  agentPubkey: string;
  ownerPubkey: string;
  relayUrl: string;
};

export type BestieListAddInput = {
  dayKey?: string | null;
  dueAt?: number | null;
  kind: BestieListKind;
  repeat?: BestieReminderRepeat | null;
  sortOrder?: number;
  sourceMessageId?: string | null;
  starred?: boolean;
  text: string;
};

export type BestieListTodoUpdateInput = {
  dayKey?: string | null;
  id: string;
  sortOrder?: number;
  starred?: boolean;
  status?: "open" | "done";
  text?: string;
};
