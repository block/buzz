/**
 * First-join alert bookkeeping for community owners/admins.
 *
 * # Why the roster snapshot is the source of truth, not the kind:8000 delta
 *
 * The relay emits a kind:8000 "member-added" delta on the invite-claim and
 * relay-admin paths, but `buzz-admin add-member` deliberately emits none
 * (`crates/buzz-admin/src/main.rs:6-13`), and kind:8000 fan-out is pod-local
 * (`fan_out_event_to_local_subscribers` never calls `publish_event`, unlike
 * `dispatch_persistent_event_inner`). The kind:13534 membership snapshot is the
 * only signal that covers every join path *and* propagates across pods, so it
 * is the correctness signal here; kind:8000 is a latency accelerator only.
 *
 * # Why a persisted ledger rather than snapshot-to-snapshot diffing
 *
 * Snapshot publication is eventual, not transactional: a failed post-commit
 * publish is repaired by the relay's periodic reconciler, so the same member
 * can first appear in a snapshot arriving up to a reconcile interval late, and
 * a reconciler-published snapshot is indistinguishable from a fresh one. Only a
 * ledger of pubkeys we have already alerted on can answer "is this new to the
 * user", which is the question the notification actually asks. The ledger also
 * absorbs kind:8000 redelivery on reconnect, where the replay filter re-sends
 * events at or after `lastSeenCreatedAt - skew` and can repeat a seen delta.
 */

import { setLocalStorageItemWithRecovery } from "@/shared/lib/localStorageQuota";

const JOIN_ALERT_STORAGE_PREFIX = "buzz-community-join-seen.v1";

/**
 * Cap on *departed* pubkeys retained per community.
 *
 * A pubkey still on the roster can never be shed: the next snapshot presents it
 * again, the ledger no longer recognizes it, and it is alerted as a fresh join
 * — on every snapshot, forever. So the cap bounds only the tail of keys that
 * have left, and the ledger's real ceiling is the roster the relay can deliver
 * (a kind:13534 snapshot larger than `BUZZ_MAX_FRAME_BYTES` never arrives).
 */
export const JOIN_ALERT_DEPARTED_MAX_ITEMS = 5_000;

export type JoinAlertLedger = {
  /**
   * Whether a roster snapshot has already been folded in for this community.
   *
   * Tracked explicitly rather than inferred from `pubkeys.length > 0`, because
   * the two are not the same proposition: a community whose only member is the
   * viewer seeds to an *empty* pubkey list (the viewer is never recorded), and
   * inferring from emptiness would then classify the first genuine join as the
   * seeding run and silently swallow the very alert this feature exists for.
   */
  seeded: boolean;
  /** Pubkeys already alerted on, oldest first. */
  pubkeys: string[];
};

export const EMPTY_JOIN_ALERT_LEDGER: JoinAlertLedger = {
  seeded: false,
  pubkeys: [],
};

export function joinAlertStorageKey(communityId: string, viewerPubkey: string) {
  return `${JOIN_ALERT_STORAGE_PREFIX}:${communityId}:${viewerPubkey}`;
}

export function normalizeJoinPubkey(pubkey: string): string {
  return pubkey.trim().toLowerCase();
}

export function readJoinAlertLedger(
  communityId: string,
  viewerPubkey: string,
): JoinAlertLedger {
  if (
    typeof window === "undefined" ||
    communityId.length === 0 ||
    viewerPubkey.length === 0
  ) {
    return EMPTY_JOIN_ALERT_LEDGER;
  }

  const rawValue = window.localStorage.getItem(
    joinAlertStorageKey(communityId, viewerPubkey),
  );
  if (!rawValue) {
    return EMPTY_JOIN_ALERT_LEDGER;
  }

  try {
    const parsed: unknown = JSON.parse(rawValue);
    if (parsed === null || typeof parsed !== "object") {
      return EMPTY_JOIN_ALERT_LEDGER;
    }

    const { pubkeys, seeded } = parsed as Partial<JoinAlertLedger>;
    if (!Array.isArray(pubkeys)) {
      return EMPTY_JOIN_ALERT_LEDGER;
    }

    return {
      // A stored ledger is by definition the residue of a snapshot we already
      // folded in, so unreadable/absent `seeded` reads as true. Defaulting the
      // other way would re-seed and drop a real join.
      seeded: seeded !== false,
      pubkeys: pubkeys.filter(
        (value): value is string => typeof value === "string",
      ),
    };
  } catch {
    return EMPTY_JOIN_ALERT_LEDGER;
  }
}

/**
 * Persist the ledger. Returns false when the write did not land.
 *
 * Routed through the quota-aware writer rather than `localStorage.setItem`:
 * this runs inside an async snapshot handler, where a raw QuotaExceededError
 * would reject before the notification is ever sent, and it would do so on
 * every subsequent snapshot too.
 */
export function writeJoinAlertLedger(
  communityId: string,
  viewerPubkey: string,
  ledger: JoinAlertLedger,
): boolean {
  if (
    typeof window === "undefined" ||
    communityId.length === 0 ||
    viewerPubkey.length === 0
  ) {
    return false;
  }

  return setLocalStorageItemWithRecovery(
    joinAlertStorageKey(communityId, viewerPubkey),
    JSON.stringify(ledger satisfies JoinAlertLedger),
  );
}

/**
 * Fold a roster snapshot into the ledger, returning the pubkeys to alert on.
 *
 * The viewer's own pubkey is never alerted on or recorded: an owner does not
 * need to be told they joined their own community.
 *
 * `alerts` is empty on the seeding run — the first snapshot for a community
 * records every existing member silently, so installing the app against an
 * established roster does not produce a notification per member.
 */
export function reconcileJoinAlertLedger({
  ledger,
  rosterPubkeys,
  viewerPubkey,
}: {
  ledger: JoinAlertLedger;
  rosterPubkeys: readonly string[];
  viewerPubkey: string;
}): { alerts: string[]; changed: boolean; ledger: JoinAlertLedger } {
  const normalizedViewer = normalizeJoinPubkey(viewerPubkey);
  const seen = new Set(ledger.pubkeys);
  const roster = new Set<string>();
  const fresh: string[] = [];

  for (const rawPubkey of rosterPubkeys) {
    const pubkey = normalizeJoinPubkey(rawPubkey);
    if (pubkey.length === 0) continue;
    if (pubkey === normalizedViewer) continue;
    roster.add(pubkey);
    if (seen.has(pubkey)) continue;
    seen.add(pubkey);
    fresh.push(pubkey);
  }

  if (fresh.length === 0 && ledger.seeded) {
    return { alerts: [], changed: false, ledger };
  }

  // Shed only pubkeys absent from the roster we were just handed. Capping the
  // whole ledger instead would evict keys that are still members, and every
  // later snapshot would then re-alert them — permanently, once the roster
  // passes the cap.
  const departed = ledger.pubkeys.filter((pubkey) => !roster.has(pubkey));
  const shedCount = departed.length - JOIN_ALERT_DEPARTED_MAX_ITEMS;
  const shed = shedCount > 0 ? new Set(departed.slice(0, shedCount)) : null;
  const retained =
    shed === null
      ? ledger.pubkeys
      : ledger.pubkeys.filter((pubkey) => !shed.has(pubkey));

  return {
    alerts: ledger.seeded ? fresh : [],
    changed: true,
    ledger: {
      seeded: true,
      pubkeys: [...retained, ...fresh],
    },
  };
}

/** Notification copy for a single first join. */
export function joinAlertTitle(communityName: string | null | undefined) {
  const trimmed = communityName?.trim();
  return trimmed && trimmed.length > 0
    ? `New member in ${trimmed}`
    : "New community member";
}

export function joinAlertBody(displayName: string) {
  return `${displayName} joined`;
}

/**
 * Most per-key notifications emitted for a single snapshot.
 *
 * Above this, one summary replaces the batch. A snapshot is a whole roster, not
 * an event per join, so a bulk import or an invite link shared into a group
 * chat lands every new key at once: without a cap that is one OS notification
 * per member (measured: a 250-key snapshot emitted 248 banners in a serial
 * loop). The cap is deliberately small — past a handful the individual
 * identities are unreadable as notifications anyway, and the useful signal is
 * that a batch arrived.
 */
export const JOIN_ALERT_MAX_INDIVIDUAL = 3;

export function joinAlertSummaryBody(count: number) {
  return `${count} new members joined`;
}
