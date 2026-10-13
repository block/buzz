// Relay-backed per-user notification preferences.
//
// Two kind-30078 app-data events, both authored by the member:
//   d=buzz:notification-prefs  NIP-44 to self. The per-type on/off switches, synced
//                              desktop <-> web (last writer wins on `updatedAt`).
//   d=buzz:push-prefs          NIP-44 to the relay's push executor key. What the relay
//                              push matcher honors: classes turned off + muted
//                              channels/DMs + muted threads. Lets a switch flipped on
//                              desktop silence the phone without reopening the PWA.
import { relayClient } from "@/shared/api/relayClient";
import {
  getRelayHttpUrl,
  nip44DecryptFromSelf,
  nip44EncryptToPeer,
  nip44EncryptToSelf,
  signRelayEvent,
} from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import {
  DEFAULT_EVENT_ALERTS_ENABLED,
  EVENT_ALERT_TYPES,
  type EventAlertType,
} from "./sound";

export const KIND_APP_DATA = 30078;
export const PREFS_D_TAG = "buzz:notification-prefs";
export const PUSH_PREFS_D_TAG = "buzz:push-prefs";

/** The four notification types that also drive push and the sound rows. */
export const SYNCED_SLOTS = [
  "dm",
  "mention",
  "thread_reply",
  "needs_action",
] as const;
export type SyncedSlot = (typeof SYNCED_SLOTS)[number];

export type SyncedTypePrefs = {
  updatedAt: number;
  slots: Record<SyncedSlot, boolean>;
  events: Record<EventAlertType, boolean>;
};

export function parseSyncedTypePrefs(raw: unknown): SyncedTypePrefs | null {
  if (!raw || typeof raw !== "object") return null;
  const r = raw as {
    v?: unknown;
    updatedAt?: unknown;
    slots?: Record<string, unknown>;
    events?: Record<string, unknown>;
  };
  if (r.v !== 1 || typeof r.updatedAt !== "number") return null;
  const slots = {} as Record<SyncedSlot, boolean>;
  for (const s of SYNCED_SLOTS) slots[s] = r.slots?.[s] !== false;
  const events = { ...DEFAULT_EVENT_ALERTS_ENABLED };
  for (const e of EVENT_ALERT_TYPES)
    if (typeof r.events?.[e] === "boolean") events[e] = r.events[e] as boolean;
  return { updatedAt: r.updatedAt, slots, events };
}

export function serializeSyncedTypePrefs(p: SyncedTypePrefs): string {
  return JSON.stringify({
    v: 1,
    updatedAt: p.updatedAt,
    slots: p.slots,
    events: p.events,
  });
}

export type PushPrefsPayload = {
  v: 1;
  off: SyncedSlot[];
  muted_channels: string[];
  muted_threads: string[];
};

const MAX_MUTES = 500;

export function buildPushPrefsPayload(
  slots: Record<SyncedSlot, boolean>,
  mutedChannelIds: Iterable<string>,
  mutedRootIds: Iterable<string>,
): PushPrefsPayload {
  return {
    v: 1,
    off: SYNCED_SLOTS.filter((s) => !slots[s]),
    muted_channels: [...new Set(mutedChannelIds)].sort().slice(0, MAX_MUTES),
    muted_threads: [...new Set(mutedRootIds)].sort().slice(0, MAX_MUTES),
  };
}

function newest(events: RelayEvent[], d: string, pubkey: string) {
  return events
    .filter(
      (e) =>
        e.kind === KIND_APP_DATA &&
        e.pubkey.toLowerCase() === pubkey &&
        e.tags.some((t) => t[0] === "d" && t[1] === d),
    )
    .sort((a, b) => b.created_at - a.created_at)[0];
}

export async function fetchSyncedTypePrefs(
  pubkey: string,
): Promise<SyncedTypePrefs | null> {
  const events = await relayClient.fetchEvents({
    kinds: [KIND_APP_DATA],
    authors: [pubkey],
    "#d": [PREFS_D_TAG],
    limit: 4,
  });
  const ev = newest(events, PREFS_D_TAG, pubkey);
  if (!ev) return null;
  try {
    return parseSyncedTypePrefs(
      JSON.parse(await nip44DecryptFromSelf(ev.content)),
    );
  } catch {
    return null;
  }
}

async function publishAppData(d: string, content: string, label: string) {
  const event = await signRelayEvent({
    kind: KIND_APP_DATA,
    content,
    tags: [["d", d]],
  });
  await relayClient.publishEvent(
    event,
    `Timed out publishing ${label}.`,
    `Failed to publish ${label}.`,
  );
}

export async function publishSyncedTypePrefs(p: SyncedTypePrefs) {
  const content = await nip44EncryptToSelf(serializeSyncedTypePrefs(p));
  await publishAppData(PREFS_D_TAG, content, "notification settings");
}

let executorPubkeyPromise: Promise<string | null> | null = null;
/** The relay's push executor pubkey from NIP-11, or null when the relay offers no push. */
export function relayPushExecutorPubkey(): Promise<string | null> {
  executorPubkeyPromise ??= (async () => {
    try {
      const base = (await getRelayHttpUrl()).replace(/\/+$/, "");
      const res = await fetch(`${base}/info`, {
        headers: { Accept: "application/nostr+json" },
      });
      if (!res.ok) return null;
      const info = (await res.json()) as {
        push?: { keys?: { pubkey?: string; current?: boolean }[] };
      };
      const key = info.push?.keys?.find((k) => k.current === true)?.pubkey;
      return typeof key === "string" && /^[0-9a-f]{64}$/.test(key) ? key : null;
    } catch {
      return null;
    }
  })();
  const p = executorPubkeyPromise;
  void p.then((v) => {
    if (v === null && executorPubkeyPromise === p) executorPubkeyPromise = null;
  });
  return p;
}

/** Publish the relay-readable push prefs. Resolves false when unsupported (no push / old desktop build). */
export async function publishPushPrefs(
  payload: PushPrefsPayload,
): Promise<boolean> {
  const executor = await relayPushExecutorPubkey();
  if (!executor) return false;
  let content: string;
  try {
    content = await nip44EncryptToPeer(executor, JSON.stringify(payload));
  } catch {
    return false;
  }
  await publishAppData(PUSH_PREFS_D_TAG, content, "push settings");
  return true;
}
