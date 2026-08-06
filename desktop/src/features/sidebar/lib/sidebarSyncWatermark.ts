/**
 * Persisted remote-head watermark for sidebar-preference sync managers.
 *
 * Each manager (sections, sort, stars, mutes) persists the highest
 * `created_at` it has ever observed from the relay under a key scoped to
 * pubkey + relay + blob type.  On the next boot the manager reads this value
 * back: if it is > 0 a remote blob has existed before and seed-publishing
 * must be skipped even when the fetch comes back empty (error, timeout, or
 * auth-race).
 *
 * Keys live in localStorage alongside the payload blobs.  They are tiny
 * (one integer string per key) and scoped so they never bleed across
 * identities, communities, or blob types.
 */

const PREFIX = "buzz-sync-watermark.v1";

/**
 * Tri-state result returned by every `fetchRemote*()` method.
 *
 * - `found`  — the relay returned an event that decrypted and parsed cleanly.
 * - `absent` — the relay was successfully queried and returned zero events
 *              (genuine first-time use on this relay).
 * - `failed` — the fetch threw (timeout, relay error, auth-race), or an event
 *              existed but could not be decrypted/parsed.  In the `failed`
 *              case, `createdAt` may be set when the event itself was readable
 *              even though its payload was not — the manager records the head
 *              so seed-publish is still blocked.
 */
export type FetchResult<T> =
  | { status: "found"; data: T; createdAt: number; eventId: string }
  | { status: "absent" }
  | { status: "failed"; createdAt?: number };

function watermarkKey(
  pubkey: string,
  blobType: string,
  relayUrl?: string,
): string {
  if (!relayUrl) return `${PREFIX}:${blobType}:${pubkey}`;
  return `${PREFIX}:${blobType}:${pubkey}:${encodeURIComponent(relayUrl)}`;
}

/** Read the persisted watermark (0 when absent or on read error). */
export function readWatermark(
  pubkey: string,
  blobType: string,
  relayUrl?: string,
): number {
  try {
    const raw = window.localStorage.getItem(
      watermarkKey(pubkey, blobType, relayUrl),
    );
    if (raw === null) return 0;
    const n = Number(raw);
    return Number.isFinite(n) && n > 0 ? n : 0;
  } catch {
    return 0;
  }
}

/**
 * Persist a new watermark if it is strictly greater than the current value.
 * Returns the value actually stored (may be the old value if `next` is not
 * newer).
 */
export function advanceWatermark(
  pubkey: string,
  blobType: string,
  next: number,
  relayUrl?: string,
): void {
  try {
    const current = readWatermark(pubkey, blobType, relayUrl);
    if (next <= current) return;
    window.localStorage.setItem(
      watermarkKey(pubkey, blobType, relayUrl),
      String(next),
    );
  } catch {
    // Ignore write failures — the in-memory lastRemoteCreatedAt still guards
    // seed-publish within this session; the watermark is belt-and-suspenders
    // across sessions.
  }
}
