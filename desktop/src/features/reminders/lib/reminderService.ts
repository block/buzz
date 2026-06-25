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

/**
 * Generate a reminder `d`-tag with 128 bits of entropy (NIP-ER line 58 MUST).
 * `crypto.randomUUID()` is UUIDv4 = only 122 random bits, so use 16 raw bytes.
 */
function randomDTag(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

function extractNotBefore(event: RelayEvent): number | undefined {
  const tag = event.tags.find((t) => t[0] === "not_before");
  return tag?.[1] ? parseNotBefore(tag[1]) : undefined;
}

/**
 * Parse a NIP-ER `not_before` tag value, mirroring the relay's strict
 * validator (NIP-ER line 60): ASCII digits only, no leading zero except "0",
 * and within `Number.MAX_SAFE_INTEGER`. Returns undefined for any value the
 * relay would reject, so the client ignores reminders the relay calls malformed.
 */
export function parseNotBefore(raw: string): number | undefined {
  if (!/^(0|[1-9][0-9]*)$/.test(raw)) return undefined;
  const val = Number(raw);
  return val <= Number.MAX_SAFE_INTEGER ? val : undefined;
}

/**
 * Validate decrypted reminder plaintext against the shape this client writes,
 * returning a typed content object or null. NIP-ER (Content section) requires
 * clients to ignore plaintext that is not a JSON object, has an unknown
 * `status`, or has a malformed target/note — so anything off-shape fails closed.
 */
export function parseReminderContent(
  plaintext: string,
): ReminderContent | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(plaintext);
  } catch {
    return null;
  }
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    return null;
  }

  const obj = parsed as Record<string, unknown>;
  if (
    obj.status !== "pending" &&
    obj.status !== "done" &&
    obj.status !== "cancelled"
  ) {
    return null;
  }
  if (obj.note !== undefined && typeof obj.note !== "string") return null;

  let target: ReminderTarget | undefined;
  if (obj.target !== undefined) {
    const parsedTarget = parseTarget(obj.target);
    if (!parsedTarget) return null;
    target = parsedTarget;
  }

  // A reminder must reference a target or carry a non-empty note.
  if (!target && !(typeof obj.note === "string" && obj.note.length > 0)) {
    return null;
  }

  return { status: obj.status, target, note: obj.note as string | undefined };
}

function parseTarget(value: unknown): ReminderTarget | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return null;
  }
  const t = value as Record<string, unknown>;
  if (
    typeof t.eventId !== "string" ||
    typeof t.channelId !== "string" ||
    typeof t.preview !== "string" ||
    typeof t.authorPubkey !== "string"
  ) {
    return null;
  }
  return {
    eventId: t.eventId,
    channelId: t.channelId,
    preview: t.preview,
    authorPubkey: t.authorPubkey,
  };
}

async function decryptReminder(event: RelayEvent): Promise<Reminder | null> {
  const dTag = extractDTag(event);
  if (!dTag) return null;

  let plaintext: string;
  try {
    plaintext = await nip44DecryptFromSelf(event.content);
  } catch {
    console.warn("[reminderService] failed to decrypt reminder:", event.id);
    return null;
  }

  const content = parseReminderContent(plaintext);
  if (!content) {
    console.warn("[reminderService] ignoring malformed reminder:", event.id);
    return null;
  }

  return {
    id: dTag,
    notBefore: extractNotBefore(event),
    content,
    createdAt: event.created_at,
    eventId: event.id,
  };
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
  const dTag = randomDTag();
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
