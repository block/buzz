import { relayClient } from "@/shared/api/relayClient";
import {
  nip44DecryptFromSelf,
  nip44EncryptToSelf,
  signRelayEvent,
} from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_EVENT_REMINDER } from "@/shared/constants/kinds";
import type {
  Reminder,
  ReminderContent,
  ReminderTarget,
} from "./reminderTypes";

// Jittered expiration for completed/cancelled reminders (30-90 days).
function jitteredExpiration(): number {
  const days = 30 + Math.floor(Math.random() * 60);
  return Math.floor(Date.now() / 1_000) + days * 86_400;
}

function extractDTag(event: RelayEvent): string | null {
  const tag = event.tags.find((t) => t[0] === "d");
  return tag?.[1] ?? null;
}

function extractNotBefore(event: RelayEvent): number | undefined {
  const tag = event.tags.find((t) => t[0] === "not_before");
  if (!tag?.[1]) return undefined;
  const val = Number.parseInt(tag[1], 10);
  return Number.isNaN(val) ? undefined : val;
}

async function decryptReminder(event: RelayEvent): Promise<Reminder | null> {
  const dTag = extractDTag(event);
  if (!dTag) return null;

  try {
    const plaintext = await nip44DecryptFromSelf(event.content);
    const content = JSON.parse(plaintext) as ReminderContent;
    return {
      id: dTag,
      notBefore: extractNotBefore(event),
      content,
      createdAt: event.created_at,
      eventId: event.id,
    };
  } catch {
    console.warn("[reminderService] failed to decrypt reminder:", event.id);
    return null;
  }
}

export async function fetchReminders(pubkey: string): Promise<Reminder[]> {
  const events = await relayClient.fetchEvents({
    kinds: [KIND_EVENT_REMINDER],
    authors: [pubkey],
    limit: 200,
  });

  const results = await Promise.all(events.map(decryptReminder));
  return results.filter((r): r is Reminder => r !== null);
}

export async function createReminder(
  target: ReminderTarget,
  notBefore: number,
  note?: string,
): Promise<RelayEvent> {
  const dTag = crypto.randomUUID();
  const content: ReminderContent = {
    target,
    note,
    status: "pending",
  };

  const ciphertext = await nip44EncryptToSelf(JSON.stringify(content));
  const tags: string[][] = [
    ["d", dTag],
    ["not_before", String(notBefore)],
  ];

  const event = await signRelayEvent({
    kind: KIND_EVENT_REMINDER,
    content: ciphertext,
    tags,
  });

  return relayClient.publishEvent(
    event,
    "Timed out creating reminder.",
    "Failed to create reminder.",
  );
}

export async function completeReminder(
  _pubkey: string,
  reminder: Reminder,
): Promise<RelayEvent> {
  const content: ReminderContent = {
    ...reminder.content,
    status: "done",
  };

  const ciphertext = await nip44EncryptToSelf(JSON.stringify(content));
  const expiration = jitteredExpiration();
  const tags: string[][] = [
    ["d", reminder.id],
    ["expiration", String(expiration)],
  ];

  const event = await signRelayEvent({
    kind: KIND_EVENT_REMINDER,
    content: ciphertext,
    createdAt: Math.max(Math.floor(Date.now() / 1_000), reminder.createdAt + 1),
    tags,
  });

  return relayClient.publishEvent(
    event,
    "Timed out completing reminder.",
    "Failed to complete reminder.",
  );
}

export async function snoozeReminder(
  _pubkey: string,
  reminder: Reminder,
  newNotBefore: number,
): Promise<RelayEvent> {
  const content: ReminderContent = {
    ...reminder.content,
    status: "pending",
  };

  const ciphertext = await nip44EncryptToSelf(JSON.stringify(content));
  const tags: string[][] = [
    ["d", reminder.id],
    ["not_before", String(newNotBefore)],
  ];

  const event = await signRelayEvent({
    kind: KIND_EVENT_REMINDER,
    content: ciphertext,
    createdAt: Math.max(Math.floor(Date.now() / 1_000), reminder.createdAt + 1),
    tags,
  });

  return relayClient.publishEvent(
    event,
    "Timed out snoozing reminder.",
    "Failed to snooze reminder.",
  );
}

export async function cancelReminder(
  _pubkey: string,
  reminder: Reminder,
): Promise<RelayEvent> {
  const content: ReminderContent = {
    ...reminder.content,
    status: "cancelled",
  };

  const ciphertext = await nip44EncryptToSelf(JSON.stringify(content));
  const expiration = jitteredExpiration();
  const tags: string[][] = [
    ["d", reminder.id],
    ["expiration", String(expiration)],
  ];

  const event = await signRelayEvent({
    kind: KIND_EVENT_REMINDER,
    content: ciphertext,
    createdAt: Math.max(Math.floor(Date.now() / 1_000), reminder.createdAt + 1),
    tags,
  });

  return relayClient.publishEvent(
    event,
    "Timed out cancelling reminder.",
    "Failed to cancel reminder.",
  );
}
