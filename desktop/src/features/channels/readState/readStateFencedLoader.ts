/**
 * NIP-RS full-state fenced loader.
 *
 * Implements the NIP-RS §Full-State Load procedure (NIP-RS.md:321-377):
 * - EOSE-established fence via `relay.subscribeFenced()` — resolves only on
 *   this subscription's own EOSE, never via the 250 ms fallback or CLOSED
 *   (NIP-RS.md:341-345). Events are delivered synchronously; no timer drain.
 * - ANY lapse (before EOSE, mid-enumeration, post-enumeration) forces
 *   `complete: false` regardless of query results.
 * - Descending `until` cursor with pinned-window check (NIP-RS.md:350-352).
 * - `T === 0` terminates complete after the pinned window is discharged —
 *   no lower standard timestamp exists, so history is fully exhausted.
 * - Coordinate deduplicated by `d` tag (greatest `created_at`, lowest id).
 * - Returns the deduped parsed records directly so the caller can process them
 *   once for merge, metadata, conflict rotation, and `maxFetchedCreatedAt`.
 */

import type { RelayClient } from "@/shared/api/relayClientSession";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_READ_STATE } from "@/shared/constants/kinds";
import {
  parseReadStateEvent,
  type ParsedReadStateEvent,
} from "@/features/channels/readState/readStateSnapshot";
import { isValidReadStateDTag } from "@/features/channels/readState/readStateFormat";

/** Number of events per enumeration band. MUST be ≥ L=2 (NIP-RS.md:347). */
const BAND_LIMIT = 500;
/** NIP-RS §Floor: relay MUST deliver ≥ L events when ≥ L exist (NIP-RS.md:360). */
const L = 2;

export type FencedLoadResult =
  | { complete: true; events: ParsedReadStateEvent[] }
  | { complete: false; events: ParsedReadStateEvent[] };

/**
 * Perform one full-state fenced enumeration for `pubkey`.
 *
 * Returns the coordinate-deduped parsed records and whether the load was proven
 * complete per the NIP-RS §Full-State Load procedure. A `complete: false`
 * result carries best-effort events for baseline display.
 */
export async function fencedEnumerationLoad(
  relay: RelayClient,
  pubkey: string,
): Promise<FencedLoadResult> {
  const baseFilter = {
    kinds: [KIND_READ_STATE],
    authors: [pubkey],
    limit: BAND_LIMIT,
  };

  // ── Step 1: Establish the fence BEFORE the first query ──────────────────
  // subscribeFenced() resolves `established` ONLY on EOSE — no fallback timer.
  // Events are delivered synchronously to `fenceEvents`; no drain timer needed.
  const fenceEvents: RelayEvent[] = [];
  let fence: Awaited<ReturnType<typeof relay.subscribeFenced>>;
  try {
    fence = await relay.subscribeFenced(baseFilter, (ev) => {
      fenceEvents.push(ev);
    });
  } catch {
    return emptyIncomplete();
  }

  if (fence.lapsed) {
    return emptyIncomplete();
  }

  // Wait for EOSE — resolves only when the relay confirms this subscription.
  await fence.established;

  if (fence.lapsed) {
    await fence.unsubscribe();
    return emptyIncomplete();
  }

  // ── Step 2: Descending enumeration ──────────────────────────────────────
  let C = 0;
  let until: number | undefined;
  const bandEvents: RelayEvent[] = [];
  let complete = false;

  while (!fence.lapsed) {
    const filter = { ...baseFilter, ...(until !== undefined ? { until } : {}) };
    let band: RelayEvent[];
    try {
      band = await relay.fetchEvents(filter);
    } catch {
      break;
    }
    if (fence.lapsed) break;

    if (band.length === 0) {
      complete = true;
      break;
    }

    if (band.length > C) C = band.length;
    for (const ev of band) bandEvents.push(ev);

    // T = lowest created_at in this band (NIP-RS.md:350).
    let T = band[0].created_at;
    for (const ev of band) if (ev.created_at < T) T = ev.created_at;

    // ── Pinned window (NIP-RS.md:350-351) ───────────────────────────────
    if (fence.lapsed) break;
    let pinned: RelayEvent[];
    try {
      pinned = await relay.fetchEvents({ ...baseFilter, since: T, until: T });
    } catch {
      break;
    }
    if (fence.lapsed) break;

    for (const ev of pinned) bandEvents.push(ev);
    if (pinned.length > C) C = pinned.length;

    if (pinned.length >= Math.max(C, L)) {
      // Pinned window not discharged — potentially incomplete (spec :351).
      break;
    }

    if (T === 0) {
      // T=0: the pinned window was just discharged (pinned.length < max(C,L)).
      // No event can have created_at < 0, so the history is fully exhausted —
      // terminate complete directly.  An `until:0` continuation would be an
      // inclusive re-fetch of the same second-zero events and cannot prove
      // anything additional (spec :352).
      complete = true;
      break;
    }

    until = T - 1;
  }

  // ── Unsubscribe fence ────────────────────────────────────────────────────
  // Events delivered synchronously — no timer drain needed.
  await fence.unsubscribe();

  // A lapse at ANY point (including after `complete` was tentatively set)
  // forces complete:false.
  if (fence.lapsed) {
    const all = [...bandEvents, ...fenceEvents];
    return buildResult(false, all, pubkey);
  }

  const all = [...bandEvents, ...fenceEvents];
  return buildResult(complete, all, pubkey);
}

/**
 * Deduplicate events by `d` tag: retain the event with the greatest
 * `created_at`; break ties by the lexicographically lowest event id.
 * Prevents superseded coordinate versions from being merged as concurrent
 * replicas (NIP-RS.md:348).
 */
export function deduplicateByCoordinate(events: RelayEvent[]): RelayEvent[] {
  const best = new Map<string, RelayEvent>();
  for (const ev of events) {
    const dTag = ev.tags.find((t) => t[0] === "d")?.[1];
    if (!dTag || !isValidReadStateDTag(dTag)) continue;
    const existing = best.get(dTag);
    if (
      !existing ||
      ev.created_at > existing.created_at ||
      (ev.created_at === existing.created_at && ev.id < existing.id)
    ) {
      best.set(dTag, ev);
    }
  }
  return [...best.values()];
}

function emptyIncomplete(): FencedLoadResult {
  return { complete: false, events: [] };
}

async function buildResult(
  complete: boolean,
  events: RelayEvent[],
  pubkey: string,
): Promise<FencedLoadResult> {
  const deduped = deduplicateByCoordinate(events);
  const parsed: ParsedReadStateEvent[] = [];
  for (const ev of deduped) {
    const p = await parseReadStateEvent(ev, pubkey);
    if (p) parsed.push(p);
  }
  return { complete, events: parsed };
}
