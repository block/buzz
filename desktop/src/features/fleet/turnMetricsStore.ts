import * as React from "react";

import { relayClient } from "@/shared/api/relayClient";
import { getIdentity } from "@/shared/api/tauriIdentity";
import { decryptObserverEvent } from "@/shared/api/tauriObserver";
import {
  listSaveSubscriptions,
  readArchivedEvents,
} from "@/shared/api/tauriArchive";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_AGENT_TURN_METRIC } from "@/shared/constants/kinds";
import { normalizePubkey } from "@/shared/lib/pubkey";
import {
  aggregateAgentTurnMetrics,
  EMPTY_USAGE_TOTALS,
  isTurnMetricPayload,
  type AgentUsageTotals,
  type TurnMetricPayload,
} from "./turnMetrics";

/**
 * Module store for per-agent turn-metric usage (kind 44200, NIP-AM).
 *
 * Sources, both existing desktop data paths — no new endpoints:
 * - The local SQLite archive (`owner_p` save subscription including 44200,
 *   seeded by `useAgentMetricArchiveSeed` or toggled in Local Archive
 *   settings), read via `read_archived_events`.
 * - The relay itself: kind 44200 is relay-persisted and `#p`-addressed to the
 *   owner (result-gated to the addressed recipient), fetched once on load and
 *   then followed live so the board updates as turns complete.
 *
 * Every raw event is decrypted through the same Tauri `decrypt_observer_event`
 * command the observer store uses (44200 shares the NIP-44 agent→owner
 * encryption scheme with kind 24200). Decrypt failures are silently dropped,
 * mirroring `ingestArchivedObserverEvents`.
 *
 * Community switching: this is a module-level singleton, so
 * `resetFleetTurnMetricsStore()` is wired into `resetCommunityState()`
 * (see `features/communities/useCommunityInit.ts`).
 */

/** Archive pages read on load (newest-first); bounds decrypt IPC on mount. */
const ARCHIVE_PAGE_SIZE = 200;
const ARCHIVE_PAGE_BUDGET = 3;
/** One-shot relay backfill window for owners without a local archive. */
const RELAY_FETCH_LIMIT = 500;

// agentKey → eventId → decrypted payload. Dedup across archive/relay/live by
// event id, so the same metric never counts twice.
const payloadsByAgent = new Map<string, Map<string, TurnMetricPayload>>();
const seenEventIds = new Set<string>();

const listeners = new Set<() => void>();

// Reference-stable snapshots for useSyncExternalStore.
const totalsCache = new Map<string, AgentUsageTotals>();

let loadPromise: Promise<void> | null = null;
let loaded = false;
let unsubscribeLive: (() => Promise<void>) | null = null;
let generation = 0;
// Monotonic change counter — a cheap useSyncExternalStore snapshot for
// consumers that read many agents' totals in one pass (the fleet summary).
let storeVersion = 0;

function notifyListeners() {
  storeVersion += 1;
  for (const listener of listeners) {
    listener();
  }
}

export function subscribeFleetTurnMetrics(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function ingestPayload(
  agentPubkey: string,
  eventId: string,
  payload: TurnMetricPayload,
): boolean {
  if (seenEventIds.has(eventId)) return false;
  seenEventIds.add(eventId);
  const key = normalizePubkey(agentPubkey);
  let agentPayloads = payloadsByAgent.get(key);
  if (!agentPayloads) {
    agentPayloads = new Map();
    payloadsByAgent.set(key, agentPayloads);
  }
  agentPayloads.set(eventId, payload);
  totalsCache.delete(key);
  return true;
}

/**
 * Decrypt and ingest one raw kind 44200 event. The event author is the agent
 * (the `#p` tag is the owner), so totals key on `event.pubkey`. Non-44200
 * kinds and undecryptable/malformed payloads are dropped silently — the same
 * fail-quiet contract as the archived-observer ingest path.
 */
async function ingestRawMetricEvent(event: RelayEvent): Promise<boolean> {
  if (event.kind !== KIND_AGENT_TURN_METRIC) return false;
  if (seenEventIds.has(event.id)) return false;
  try {
    const decoded = await decryptObserverEvent(event);
    if (!isTurnMetricPayload(decoded)) return false;
    return ingestPayload(event.pubkey, event.id, decoded);
  } catch {
    return false;
  }
}

async function loadArchivedMetrics(ownerPubkey: string): Promise<boolean> {
  let subs: Awaited<ReturnType<typeof listSaveSubscriptions>>;
  try {
    subs = await listSaveSubscriptions();
  } catch {
    return false;
  }
  const hasMetricSub = subs.some(
    (sub) =>
      sub.scopeType === "owner_p" &&
      sub.scopeValue === ownerPubkey &&
      sub.kinds.includes(KIND_AGENT_TURN_METRIC),
  );
  if (!hasMetricSub) return false;

  let changed = false;
  let before: { createdAt: number; id: string } | null = null;
  for (let page = 0; page < ARCHIVE_PAGE_BUDGET; page += 1) {
    let events: RelayEvent[];
    try {
      events = await readArchivedEvents("owner_p", ownerPubkey, {
        kinds: [KIND_AGENT_TURN_METRIC],
        before,
        limit: ARCHIVE_PAGE_SIZE,
      });
    } catch {
      break;
    }
    for (const event of events) {
      if (await ingestRawMetricEvent(event)) changed = true;
    }
    if (events.length < ARCHIVE_PAGE_SIZE) break;
    const oldest = events[events.length - 1];
    before = { createdAt: oldest.created_at, id: oldest.id };
  }
  return changed;
}

async function loadRelayMetrics(ownerPubkey: string): Promise<boolean> {
  let events: RelayEvent[];
  try {
    events = await relayClient.fetchEvents({
      kinds: [KIND_AGENT_TURN_METRIC],
      "#p": [ownerPubkey],
      limit: RELAY_FETCH_LIMIT,
    });
  } catch {
    return false;
  }
  let changed = false;
  for (const event of events) {
    if (await ingestRawMetricEvent(event)) changed = true;
  }
  return changed;
}

/**
 * One-shot load of archived + relay-persisted turn metrics for the current
 * identity, then a live follow subscription so new turn completions update
 * totals without a refetch. Concurrent callers share one promise; a failed
 * load clears it so the next mount retries.
 */
export function ensureFleetTurnMetricsLoaded(): Promise<void> {
  if (loaded) return Promise.resolve();
  if (loadPromise) return loadPromise;

  const activeGeneration = generation;
  loadPromise = (async () => {
    const identity = await getIdentity();
    if (activeGeneration !== generation) return;
    const ownerPubkey = identity.pubkey;

    const [archiveChanged, relayChanged] = await Promise.all([
      loadArchivedMetrics(ownerPubkey),
      loadRelayMetrics(ownerPubkey),
    ]);
    if (activeGeneration !== generation) return;
    if (archiveChanged || relayChanged) {
      notifyListeners();
    }

    // Live follow: limit 0 = no replay, new events only (backfill above
    // already covered history; dedup by event id makes overlap harmless).
    const unsubscribe = await relayClient.subscribeLive(
      {
        kinds: [KIND_AGENT_TURN_METRIC],
        "#p": [ownerPubkey],
        limit: 0,
      },
      (event) => {
        void ingestRawMetricEvent(event).then((changed) => {
          if (changed && activeGeneration === generation) {
            notifyListeners();
          }
        });
      },
    );
    if (activeGeneration !== generation) {
      await unsubscribe();
      return;
    }
    unsubscribeLive = unsubscribe;
    loaded = true;
  })()
    .catch((error) => {
      if (activeGeneration === generation) {
        console.warn("[fleet] turn metric load failed:", error);
      }
    })
    .finally(() => {
      if (activeGeneration === generation) {
        loadPromise = null;
      }
    });

  return loadPromise;
}

const EMPTY_TOTALS = EMPTY_USAGE_TOTALS;

/** Usage totals for one agent. Reference-stable until its payloads change. */
export function getAgentUsageTotals(
  agentPubkey: string | null | undefined,
): AgentUsageTotals {
  if (!agentPubkey) return EMPTY_TOTALS;
  const key = normalizePubkey(agentPubkey);
  const cached = totalsCache.get(key);
  if (cached) return cached;
  const payloads = payloadsByAgent.get(key);
  if (!payloads || payloads.size === 0) return EMPTY_TOTALS;
  const totals = aggregateAgentTurnMetrics([...payloads.values()]);
  totalsCache.set(key, totals);
  return totals;
}

/** Hook: usage totals for one agent; loads the store on first use. */
export function useAgentUsageTotals(
  agentPubkey: string | null | undefined,
): AgentUsageTotals {
  const getSnapshot = React.useCallback(
    () => getAgentUsageTotals(agentPubkey),
    [agentPubkey],
  );
  React.useEffect(() => {
    void ensureFleetTurnMetricsLoaded();
  }, []);
  return React.useSyncExternalStore(subscribeFleetTurnMetrics, getSnapshot);
}

function getFleetMetricsVersion(): number {
  return storeVersion;
}

/**
 * Hook: monotonic version that bumps whenever new turn metrics are ingested.
 * Lets a component memo a multi-agent aggregation over `getAgentUsageTotals`
 * without subscribing once per agent. Loads the store on first use.
 */
export function useFleetMetricsVersion(): number {
  React.useEffect(() => {
    void ensureFleetTurnMetricsLoaded();
  }, []);
  return React.useSyncExternalStore(
    subscribeFleetTurnMetrics,
    getFleetMetricsVersion,
  );
}

/**
 * E2E-only: inject decoded turn-metric payloads directly, bypassing the
 * archive/relay/decrypt pipeline — mirrors `injectObserverEventsForE2E`.
 * Never call from production code.
 */
export function injectFleetTurnMetricsForE2E(
  agentPubkey: string,
  payloads: TurnMetricPayload[],
) {
  let injected = 0;
  for (const payload of payloads) {
    if (!isTurnMetricPayload(payload)) continue;
    injected += 1;
    ingestPayload(
      agentPubkey,
      `e2e-${normalizePubkey(agentPubkey)}-${seenEventIds.size}-${injected}`,
      payload,
    );
  }
  if (injected > 0) notifyListeners();
}

/** Community-switch reset (see resetCommunityState in useCommunityInit). */
export function resetFleetTurnMetricsStore() {
  generation += 1;
  const unsubscribe = unsubscribeLive;
  unsubscribeLive = null;
  loadPromise = null;
  loaded = false;
  payloadsByAgent.clear();
  seenEventIds.clear();
  totalsCache.clear();
  notifyListeners();
  void unsubscribe?.();
}
