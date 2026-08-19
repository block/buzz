import { invokeTauri } from "@/shared/api/tauri";

/**
 * A pending scheduled-message delivery in the local queue. Serialized
 * snake_case to match the CLI's shared store at
 * `<app-data>/scheduled/scheduled-messages.json`.
 */
export type ScheduledMessage = {
  id: string;
  channelId: string;
  content: string;
  kind?: number | null;
  replyTo?: string | null;
  broadcast?: boolean | null;
  mentions: string[];
  scheduledAt: number;
  createdAt: number;
};

export type ScheduleMessageInput = {
  channelId: string;
  content: string;
  replyTo?: string | null;
  mentions?: string[];
  /** ISO8601 / RFC 3339 timestamp for future delivery. */
  scheduledAt: string;
};

type RawScheduledMessage = {
  id: string;
  channel_id: string;
  content: string;
  kind?: number | null;
  reply_to?: string | null;
  broadcast?: boolean | null;
  mentions?: string[];
  scheduled_at: number;
  created_at: number;
};

function fromRawScheduledMessage(raw: RawScheduledMessage): ScheduledMessage {
  return {
    id: raw.id,
    channelId: raw.channel_id,
    content: raw.content,
    kind: raw.kind ?? null,
    replyTo: raw.reply_to ?? null,
    broadcast: raw.broadcast ?? null,
    mentions: raw.mentions ?? [],
    scheduledAt: raw.scheduled_at,
    createdAt: raw.created_at,
  };
}

/** List all pending scheduled messages, newest first. */
export async function listScheduledMessages(): Promise<ScheduledMessage[]> {
  return (await invokeTauri<RawScheduledMessage[]>("scheduled_list")).map(
    fromRawScheduledMessage,
  );
}

/** Enqueue a message for delivery at the given time. */
export async function scheduleMessage(
  input: ScheduleMessageInput,
): Promise<ScheduledMessage> {
  return fromRawScheduledMessage(
    await invokeTauri<RawScheduledMessage>("scheduled_enqueue", {
      input: {
        channelId: input.channelId,
        content: input.content,
        replyTo: input.replyTo ?? null,
        mentions: input.mentions ?? [],
        scheduledAt: input.scheduledAt,
      },
    }),
  );
}

/** Cancel a pending scheduled message; returns the removed entry. */
export async function cancelScheduledMessage(
  id: string,
): Promise<ScheduledMessage> {
  return fromRawScheduledMessage(
    await invokeTauri<RawScheduledMessage>("scheduled_cancel", { id }),
  );
}

/**
 * Atomically remove and return every scheduled message that is due now. The
 * delivery loop calls this once per sweep; entries it fails to deliver are
 * re-enqueued so a later sweep retries them.
 */
export async function takeDueScheduledMessages(): Promise<ScheduledMessage[]> {
  return (await invokeTauri<RawScheduledMessage[]>("scheduled_take_due")).map(
    fromRawScheduledMessage,
  );
}

/**
 * Re-persist an entry the delivery loop already took but failed to deliver.
 * Stored verbatim (past `scheduled_at` included) so the next sweep retries it.
 */
export async function reenqueueScheduledMessage(
  message: ScheduledMessage,
): Promise<void> {
  await invokeTauri("scheduled_reenqueue", {
    message: {
      id: message.id,
      channelId: message.channelId,
      content: message.content,
      replyTo: message.replyTo,
      mentions: message.mentions,
      scheduledAt: message.scheduledAt,
      createdAt: message.createdAt,
    },
  });
}

/** Earliest scheduled timestamp still pending, if any. */
export async function nextDueScheduledMessage(): Promise<number | null> {
  return invokeTauri<number | null>("scheduled_next_due");
}
