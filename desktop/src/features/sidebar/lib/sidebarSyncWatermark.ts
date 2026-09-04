/**
 * Persisted remote-head watermark for sidebar-preference sync managers.
 *
 * Each manager (sections, sort, stars, mutes, project membership) persists the highest
 * `created_at` it has ever observed from the relay under a key scoped to
 * pubkey + relay + blob type.  On the next boot the manager reads this value
 * back: if it is > 0 a remote blob has existed before and seed-publishing
 * must be skipped even when the fetch comes back empty (error, timeout, or
 * auth-race).
 *
 * Keys live in localStorage alongside the payload blobs.  They are tiny
 * (one integer string per key) and scoped so they never bleed across
 * identities, communities, or blob types.
 *
 * `relayUrl` is always required — a pubkey-only fallback is not safe because
 * a head seen on relay A would suppress legitimate first-time seeding on
 * relay B.  The URL is normalised (trimmed, trailing slash stripped,
 * lower-cased) before being embedded in the key so the same relay written
 * two ways never produces two different keys.
 */

import { normalizeRelayUrl } from "@/shared/lib/normalizeRelayUrl";

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
  relayUrl: string,
): string {
  return `${PREFIX}:${blobType}:${pubkey}:${encodeURIComponent(normalizeRelayUrl(relayUrl))}`;
}

/** Read the persisted watermark (0 when absent or on read error). */
export function readWatermark(
  pubkey: string,
  blobType: string,
  relayUrl: string,
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
 * Absence or error never lowers the watermark (monotonic).
 */
export function advanceWatermark(
  pubkey: string,
  blobType: string,
  relayUrl: string,
  next: number,
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

/** Result returned by `bootstrap()` — the hook acts on this without publishing. */
export type BootstrapResult<T> =
  | { action: "apply-remote"; data: T }
  | { action: "hold" };

/**
 * Shared boot policy for all sidebar-preference sync managers.
 *
 * Each manager calls this from its `bootstrap()` method, supplying its
 * surface-specific fetch, publish, and local-store accessors.  The full
 * decision lives here once so that a mutation to any one surface cannot
 * escape via a per-manager copy.
 *
 * Policy:
 *   - `found`                           → return `apply-remote`; hook applies data.
 *   - `failed`                          → hold; seed-publish blocked (error or unreadable event).
 *   - `absent` + `lastHead > 0`         → hold; relay blob seen before, absence may be transient.
 *   - `absent` + `lastHead === 0` + non-empty local → call `publishFn(local)`; return `hold`.
 *   - `absent` + `lastHead === 0` + empty local     → hold; nothing to seed.
 */
export function runBootstrap<TRemote, TLocal>({
  fetchResult,
  lastHead,
  localStore,
  isLocalNonEmpty,
  publishFn,
}: {
  fetchResult: FetchResult<TRemote>;
  lastHead: number;
  localStore: TLocal;
  isLocalNonEmpty: (store: TLocal) => boolean;
  publishFn: (store: TLocal) => void;
}): BootstrapResult<TRemote> {
  if (fetchResult.status === "found") {
    return { action: "apply-remote", data: fetchResult.data };
  }
  if (fetchResult.status === "absent" && lastHead === 0) {
    if (isLocalNonEmpty(localStore)) {
      publishFn(localStore);
    }
  }
  return { action: "hold" };
}
