// NIP-PL push lease (kind 30350) for the `buzz-web` profile. Pure helpers, unit-tested.
// The relay decides who gets woken (and enforces channel access); the lease only says which
// events this member wants pushed and to which gateway grant.
import type { PushPrefs } from "./env";

export const KIND_PUSH_LEASE = 30350;
export const WEB_APP_PROFILE = "buzz-web";
export const WEB_TRANSPORT = "webpush";
const MESSAGE_KINDS = [9, 40002, 45001, 45003];
const APPROVAL_KINDS = [46010, 46011, 46012];

export type PushDescriptor = {
  origin: string;
  executorKeyId: string;
  executorPubkey: string;
  maxLeaseTtl: number;
};

/** Validate the relay's NIP-11 `push` descriptor; null when web push isn't offered. */
export function parsePushDescriptor(info: unknown): PushDescriptor | null {
  const push = (info as { push?: Record<string, unknown> } | null)?.push;
  if (!push || typeof push !== "object") return null;
  const profiles = Array.isArray(push.app_profiles)
    ? (push.app_profiles as { id?: string; transport?: string }[])
    : [];
  if (
    !profiles.some(
      (p) => p.id === WEB_APP_PROFILE && p.transport === WEB_TRANSPORT,
    )
  )
    return null;
  const keys = Array.isArray(push.keys)
    ? (push.keys as { id?: string; pubkey?: string; current?: boolean }[])
    : [];
  const current = keys.filter((k) => k.current === true);
  if (current.length !== 1) return null;
  const { id, pubkey } = current[0];
  if (
    typeof id !== "string" ||
    !id ||
    typeof pubkey !== "string" ||
    !/^[0-9a-f]{64}$/.test(pubkey)
  )
    return null;
  if (typeof push.origin !== "string" || !/^wss?:\/\//.test(push.origin))
    return null;
  const ttl = Number(
    (push.limitation as { max_lease_ttl?: number } | undefined)
      ?.max_lease_ttl ?? 0,
  );
  return {
    origin: push.origin,
    executorKeyId: id,
    executorPubkey: pubkey,
    maxLeaseTtl: ttl > 0 ? ttl : 2592000,
  };
}

type Sub = {
  filter: Record<string, unknown>;
  class: string;
  ignore: Record<string, unknown>[];
  /** Relay-evaluated participation scope (kinds-only filter). */
  scope?: "dm" | "thread";
};

/**
 * Lease subscriptions for the member's enabled notification types.
 * DMs and thread replies are matched relay-side (DM channel membership /
 * thread participation), so new DMs and threads need no re-registration.
 */
export function leaseSubscriptions(me: string, prefs: PushPrefs): Sub[] {
  const subs: Sub[] = [];
  if (prefs.mention)
    subs.push({
      filter: { kinds: MESSAGE_KINDS, "#p": [me] },
      class: "mention",
      ignore: [{ kinds: MESSAGE_KINDS, authors: [me] }],
    });
  if (prefs.needs_action)
    subs.push({
      filter: { kinds: APPROVAL_KINDS, "#p": [me] },
      class: "needs_action",
      ignore: [],
    });
  if (prefs.dm)
    subs.push({
      filter: { kinds: [9, 40002] },
      class: "dm",
      ignore: [],
      scope: "dm",
    });
  if (prefs.thread_reply)
    subs.push({
      filter: { kinds: [9, 40002] },
      class: "thread_reply",
      ignore: [],
      scope: "thread",
    });
  return subs;
}

export function leasePlaintext(args: {
  descriptor: PushDescriptor;
  generation: number;
  grant: string | null; // null => inactive (turn off)
  subscriptions: Sub[];
}): string {
  const { descriptor, generation, grant, subscriptions } = args;
  if (!grant || subscriptions.length === 0)
    return JSON.stringify({
      v: 1,
      origin: descriptor.origin,
      generation,
      active: false,
    });
  return JSON.stringify({
    v: 1,
    origin: descriptor.origin,
    app_profile: WEB_APP_PROFILE,
    transport: WEB_TRANSPORT,
    endpoint: grant,
    generation,
    active: true,
    subscriptions,
  });
}

export function leaseTags(
  installationId: string,
  descriptor: PushDescriptor,
  expiration: number,
): string[][] {
  return [
    ["d", installationId],
    ["expiration", String(expiration)],
    ["exec", descriptor.executorKeyId],
    ["alt", "Buzz web push lease"],
  ];
}
