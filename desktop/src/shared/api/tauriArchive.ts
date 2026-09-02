import { KIND_AGENT_TURN_METRIC } from "@/shared/constants/kinds";

import { invokeTauri } from "./tauri";

// ── Agent usage wire types (NIP-AM, `get_agent_usage_series`) ────────────────
//
// Mirrors `desktop-src-tauri/src/archive/agent_usage.rs` field-for-field.
// Token counters cross the Tauri boundary as decimal strings (JS cannot
// exactly represent the full `u64` range); parse with `BigInt(...)` in the
// feature slice, never `Number(...)`.

export type UsageField = { value: string | null; incomplete: boolean };
export type CostField = { value: number | null; incomplete: boolean };

export type ReportedUsage = {
  inputTokens: UsageField;
  outputTokens: UsageField;
  totalTokens: UsageField;
  estimatedCostUsd: CostField;
  /**
   * Cache-read (served) token count. A `null` value with `incomplete: false`
   * means no events in this scope reported the field. A non-empty scope where
   * all events had absent cache-read tokens produces `incomplete: true`
   * (unknown, not zero).
   */
  cacheReadTokens: UsageField;
  /**
   * Cache-write (creation) token count. Same absence semantics as
   * `cacheReadTokens`.
   */
  cacheWriteTokens: UsageField;
  /**
   * Input tokens minus cache-served and cache-write subsets. Computed only
   * when all three inputs are complete and the arithmetic succeeds
   * (`cacheRead + cacheWrite ≤ input`). Otherwise `incomplete: true`.
   */
  freshInputTokens: UsageField;
};

export type AgentUsageSeriesBucket = {
  start: number;
  end: number;
  usage: ReportedUsage;
  reportCount: number;
  hasUnknownUsage: boolean;
};

export type AgentUsageModel = {
  harness: string | null;
  model: string | null;
  usage: ReportedUsage;
  reportCount: number;
  hasUnknownUsage: boolean;
};

export type AgentUsage = {
  agentPubkey: string;
  usage: ReportedUsage;
  buckets: AgentUsageSeriesBucket[];
  models: AgentUsageModel[];
  reportCount: number;
  hasUnknownUsage: boolean;
};

export type AgentUsageCoverage = {
  firstArchivedAt: number | null;
  lastArchivedAt: number | null;
  firstReportedAt: number | null;
  lastReportedAt: number | null;
  reportCount: number;
  invalidReportCount: number;
  hasUnknownUsage: boolean;
};

export type AgentUsageSeries = {
  collectionEnabled: boolean;
  buckets: AgentUsageSeriesBucket[];
  agents: AgentUsage[];
  coverage: AgentUsageCoverage;
  /**
   * A13: `null` when the request had no `agentPubkey` filter; otherwise
   * `true` iff at least one surviving `agent_metric_index` row (either
   * `parseStatus`) exists for that author, independent of the requested
   * bucket window. Drives profile focused-view eligibility for historical
   * agents whose only evidence falls outside the current 7d/30d window.
   */
  hasArchivedEvidence: boolean | null;
};

export type AgentUsageSeriesRequest = {
  /**
   * Exact local-midnight Unix-second boundaries, inclusive start/exclusive
   * end per adjacent pair. Exactly 8 entries (7 buckets) or 31 entries (30
   * buckets) — build with the feature slice's DST-safe boundary helper,
   * never `N * 86_400`.
   */
  bucketBoundaries: number[];
  /** Normalized 64-hex author filter for the profile drill-in, or omit for the overview. */
  agentPubkey?: string;
};

// ── Wire-shape types (raw Tauri responses) ───────────────────────────────────

/**
 * `list_save_subscriptions` returns rows directly from SQLite.
 * The `kinds` column is stored as a JSON text string (e.g. `"[9,40002]"`),
 * NOT a number array — it must be decoded before use.
 */
type RawSaveSubscription = {
  identity_pubkey: string;
  relay_url: string;
  scope_type: string;
  scope_value: string;
  /** JSON-encoded integer array, e.g. `"[9,40002]"`. */
  kinds: string;
  created_at: number;
};

// ── Public types ─────────────────────────────────────────────────────────────

export type ScopeType = "channel_h" | "owner_p" | "referenced_e";

export type SaveSubscription = {
  identityPubkey: string;
  relayUrl: string;
  scopeType: ScopeType;
  scopeValue: string;
  kinds: number[];
  createdAt: number;
};

export type ArchiveBatchResult = {
  persisted: number;
  /**
   * Newly-indexed `agent_metric_index` rows (valid or invalid) written by
   * this call. A re-ingested duplicate event does not increment this even
   * when `persisted` counts it, because the index row for that id was
   * already written by whichever earlier batch first saw it. Missing on
   * the wire (older/mocked responses) decodes as `0` — see
   * `decodeArchiveBatchResult`.
   */
  persistedAgentMetrics: number;
  dropped: number;
};

/**
 * Rust sends camelCase (`#[serde(rename_all = "camelCase")]` on
 * `ArchiveBatchResult`), but decode defensively rather than trust every
 * caller (including mocks/tests) to supply every field.
 */
function decodeArchiveBatchResult(
  raw: Partial<ArchiveBatchResult>,
): ArchiveBatchResult {
  return {
    persisted: raw.persisted ?? 0,
    persistedAgentMetrics: raw.persistedAgentMetrics ?? 0,
    dropped: raw.dropped ?? 0,
  };
}

// ── Agent-metrics-change notifier ────────────────────────────────────────────

/**
 * Module-level notifier for newly persisted agent turn metrics (kind 44200).
 * `useAgentUsageSeries` subscribes to this to invalidate its query without
 * polling. Two producers: a kind-44200 subscription mutation succeeding here
 * (`collectionEnabled` is part of the usage query result), and the native
 * archive sync task persisting new metric rows — that one arrives as the
 * `archive-agent-metrics-changed` Tauri event, bridged by
 * `useArchiveAgentMetricsBridge`, since the batch it belongs to no longer
 * passes through JS.
 */
const agentMetricsChangeListeners = new Set<() => void>();

export function onAgentMetricsChanged(listener: () => void): () => void {
  agentMetricsChangeListeners.add(listener);
  return () => agentMetricsChangeListeners.delete(listener);
}

export function notifyAgentMetricsChanged(): void {
  for (const listener of agentMetricsChangeListeners) {
    listener();
  }
}

// ── Decoder ──────────────────────────────────────────────────────────────────

function decodeRawSubscription(raw: RawSaveSubscription): SaveSubscription {
  let kinds: number[] = [];
  try {
    const parsed = JSON.parse(raw.kinds);
    if (
      Array.isArray(parsed) &&
      parsed.every((k) => typeof k === "number" && Number.isFinite(k))
    ) {
      kinds = parsed as number[];
    } else {
      console.warn(
        "[tauriArchive] malformed kinds JSON (not number[]):",
        raw.kinds,
      );
    }
  } catch {
    console.warn("[tauriArchive] failed to parse kinds JSON:", raw.kinds);
  }
  return {
    identityPubkey: raw.identity_pubkey,
    relayUrl: raw.relay_url,
    scopeType: raw.scope_type as ScopeType,
    scopeValue: raw.scope_value,
    kinds,
    createdAt: raw.created_at,
  };
}

// ── API wrappers ─────────────────────────────────────────────────────────────

/**
 * Returns `true` when observer-feed archive is enabled by default.
 *
 * Always returns `true` — archive defaults to enabled for all builds.
 * The frontend calls this every startup to decide whether to reconcile
 * the `owner_p` subscription for kind 24200 (observer frames).
 */
export async function observerArchiveDefaultEnabled(): Promise<boolean> {
  return invokeTauri<boolean>("observer_archive_default_enabled");
}

/**
 * Returns `true` when agent-turn-metric archive is enabled by default.
 *
 * Always returns `true` — archive defaults to enabled for all builds.
 * The frontend calls this once at startup to decide whether to auto-seed
 * an `owner_p` [44200] subscription for new identities.
 */
export async function agentMetricArchiveDefaultEnabled(): Promise<boolean> {
  return invokeTauri<boolean>("agent_metric_archive_default_enabled");
}

/**
 * Atomically merge `kind` into the `owner_p` save subscription for the
 * current identity + relay.
 *
 * Performs a read-modify-write inside a single SQLite transaction on the Rust
 * side, so concurrent callers (e.g. observer seed + metric seed racing on
 * first run) cannot clobber each other's kind.
 *
 * Called by both `useObserverArchiveSeed` and `useAgentMetricArchiveSeed`
 * instead of the former list → merge-in-TS → create pattern.
 */
export async function mergeSaveSubscriptionKinds(kind: number): Promise<void> {
  await invokeTauri("merge_save_subscription_kinds", { kind });
  // `collectionEnabled` is part of the usage query result — toggling kind
  // 44200 on must invalidate mounted usage queries. Other kinds don't affect
  // usage state.
  if (kind === KIND_AGENT_TURN_METRIC) {
    notifyAgentMetricsChanged();
  }
}

/**
 * Atomically remove `kind` from the `owner_p` save subscription for the
 * current identity + relay.
 *
 * Mirrors `mergeSaveSubscriptionKinds`: reads existing kinds, removes `kind`,
 * then deletes the row if the list becomes empty or updates it otherwise.
 * Uses `BEGIN IMMEDIATE` on the Rust side for the same reason as the merge
 * path — concurrent toggle-OFF callers serialize rather than racing.
 *
 * Called by toggle-OFF handlers in `LocalArchiveSettingsCard` for both
 * kind 24200 and kind 44200, replacing the former TS-side read-modify-overwrite
 * that would drop the *other* kind if `subs` state was stale.
 */
export async function removeSaveSubscriptionKind(kind: number): Promise<void> {
  await invokeTauri("remove_save_subscription_kind", { kind });
  if (kind === KIND_AGENT_TURN_METRIC) {
    notifyAgentMetricsChanged();
  }
}

/**
 * Create a save subscription.
 * Runs an access probe on the backend (channel membership, event readability).
 * `kinds` is sent as a plain number array — Tauri serializes it correctly.
 */
export async function createSaveSubscription(
  scopeType: ScopeType,
  scopeValue: string,
  kinds: number[],
): Promise<void> {
  await invokeTauri("create_save_subscription", {
    scopeType,
    scopeValue,
    kinds,
  });
}

// ── Native archive sync lifecycle ────────────────────────────────────────────

/**
 * Monotonic lease counter for archive-sync lifecycle commands.
 *
 * Allocated synchronously in the renderer, before `invoke`, because effect
 * execution order IS intent order and it is the only place that ordering is
 * free. Tauri commands complete in an unconstrained order, so a token minted by
 * the backend records which call reached the mutex first — not which the app
 * actually wants. A remount whose `start` is delayed past the newer effect's
 * would otherwise mint the newest token for the stalest caller, whose cleanup
 * then holds a valid warrant to cancel the live task.
 *
 * This counter is realm-scoped: it resets to zero whenever the renderer
 * reloads, while the backend's mark persists for the life of the Tauri
 * process. The epoch below is what orders those successive realms.
 */
let archiveSyncLease = 0;

/** Allocates the next lease. Call synchronously in effect order. */
export function nextArchiveSyncLease(): number {
  archiveSyncLease += 1;
  return archiveSyncLease;
}

/**
 * Announces this renderer realm and resolves its epoch.
 *
 * Must be awaited before any lifecycle command is issued: an unawaited
 * announcement is just another racing `invoke`, which would order
 * announcements rather than realms and reintroduce the arrival-order bug one
 * level up.
 *
 * Only the main window announces. Archive sync is app-global and
 * main-window-owned — the same rule that keeps the main window the owner of
 * microphone capture. Epochs order realms in time; a companion window is a
 * second realm in space, and a newest-wins clock cannot model two concurrent
 * owners (the companion's cleanup would cancel the live main-window task).
 */
export async function announceArchiveSyncEpoch(): Promise<number> {
  return await invokeTauri<number>("announce_archive_sync_epoch");
}

/**
 * Start the backend archive sync task for the current identity.
 *
 * Idempotent per identity + relay. Must only be called after observer
 * reconciliation resolves — see `useArchiveSync` for why the backend cannot
 * gate itself. The backend ignores a mark older than the newest it has seen.
 */
export async function startArchiveSync(
  epoch: number,
  lease: number,
): Promise<void> {
  await invokeTauri("start_archive_sync", { epoch, lease });
}

/** Stop the backend archive sync task. Ignored if `(epoch, lease)` is stale. */
export async function stopArchiveSync(
  epoch: number,
  lease: number,
): Promise<void> {
  await invokeTauri("stop_archive_sync", { epoch, lease });
}

/**
 * List all save subscriptions for the current identity + relay.
 * Decodes the raw `kinds` string column into `number[]`.
 */
export async function listSaveSubscriptions(): Promise<SaveSubscription[]> {
  const rows = await invokeTauri<RawSaveSubscription[]>(
    "list_save_subscriptions",
  );
  return rows.map(decodeRawSubscription);
}

/**
 * Delete a save subscription.
 * Returns `true` if a row was removed, `false` if it didn't exist.
 */
export async function deleteSaveSubscription(
  scopeType: ScopeType,
  scopeValue: string,
): Promise<boolean> {
  const removed = await invokeTauri<boolean>("delete_save_subscription", {
    scopeType,
    scopeValue,
  });
  return removed;
}

/**
 * Archive a batch of event candidates.
 *
 * Wire-shape note (verified against Rust source at `archive/mod.rs`):
 * - `ArchiveCandidate` has no `#[serde(rename_all)]`, so struct field names
 *   are verbatim: `raw_event_json`, `matched_scope`.
 * - `MatchedScope` field names are also verbatim: `scope_type`, `scope_value`.
 * - `ScopeType` enum has `#[serde(rename_all = "snake_case")]`: values are
 *   `"channel_h"`, `"owner_p"`, `"referenced_e"`.
 * - Tauri 2 only camelCases top-level command arg names, NOT nested struct
 *   fields — so `candidates` is passed as-is, with snake_case field names.
 */
export async function archiveEvents(
  candidates: Array<{
    rawEventJson: string;
    matchedScope: { scopeType: ScopeType; scopeValue: string };
  }>,
): Promise<ArchiveBatchResult> {
  const raw = await invokeTauri<Partial<ArchiveBatchResult>>("archive_events", {
    candidates: candidates.map((c) => ({
      raw_event_json: c.rawEventJson,
      matched_scope: {
        scope_type: c.matchedScope.scopeType,
        scope_value: c.matchedScope.scopeValue,
      },
    })),
  });
  return decodeArchiveBatchResult(raw);
}

/**
 * Read a paginated page of archived kind 24200 (observer) events for a
 * specific channel, using the `observer_channel_index`.
 *
 * Only returns frames whose `channelId` was successfully decrypted and
 * matched this channel. Frames with null/decrypt-failed channelId are
 * excluded (Will's (a) ruling). Compound cursor + short-page exhaustion
 * signal work identically to `readArchivedEvents`.
 */
export async function readArchivedObserverEventsForChannel(
  channelId: string,
  opts?: {
    before?: { createdAt: number; id: string } | null;
    limit?: number;
  },
): Promise<import("@/shared/api/types").RelayEvent[]> {
  const rawRows = await invokeTauri<string[]>(
    "read_archived_observer_events_for_channel",
    {
      channelId,
      beforeCreatedAt: opts?.before?.createdAt ?? null,
      beforeId: opts?.before?.id ?? null,
      limit: opts?.limit ?? null,
    },
  );
  return rawRows
    .map((raw) => {
      try {
        return JSON.parse(raw) as import("@/shared/api/types").RelayEvent;
      } catch {
        console.warn(
          "[tauriArchive] failed to parse archived observer raw_json:",
          raw,
        );
        return null;
      }
    })
    .filter((e): e is import("@/shared/api/types").RelayEvent => e !== null);
}

/**
 * Index one or more archived observer frames by channelId.
 *
 * `channelId` is nullable: pass `null` for frames that are unscoped,
 * malformed, or whose payload could not be decrypted. Null rows are written
 * as a processed-state marker so re-runs skip them; they are never returned
 * by channel-scoped reads (which filter `channel_id = ?`).
 *
 * Idempotent — already-indexed frames are silently skipped.
 */
export async function indexObserverChannelId(
  entries: Array<{
    eventId: string;
    channelId: string | null;
    createdAt: number;
  }>,
): Promise<void> {
  if (entries.length === 0) return;
  await invokeTauri("index_observer_channel_id", {
    entries: entries.map((e) => ({
      event_id: e.eventId,
      channel_id: e.channelId,
      created_at: e.createdAt,
    })),
  });
}

/**
 * Return all `owner_p` kind 24200 archived event rows not yet indexed.
 *
 * Used by the one-shot backfill driver. Returns raw Nostr event JSON plus
 * event id and created_at for each row so the caller can decrypt and index.
 */
export async function readUnindexedObserverRows(): Promise<
  Array<{ id: string; rawJson: string; createdAt: number }>
> {
  const rows = await invokeTauri<
    Array<{ id: string; raw_json: string; created_at: number }>
  >("read_unindexed_observer_rows");
  return rows.map((r) => ({
    id: r.id,
    rawJson: r.raw_json,
    createdAt: r.created_at,
  }));
}

/**
 * Read the locally archived NIP-AM usage series for the active identity +
 * relay (Rev 3 frozen contract). Rust owns identity/relay scoping, request
 * validation, backfill-before-read, and the accounting ladder — this is a
 * thin typed wrapper with no client-side logic.
 */
export async function getAgentUsageSeries(
  request: AgentUsageSeriesRequest,
): Promise<AgentUsageSeries> {
  return invokeTauri<AgentUsageSeries>("get_agent_usage_series", { request });
}

/**
 * Read a paginated page of archived raw events for a scope.
 *
 * Returns at most `limit` raw Nostr events (default 50) in newest-first order.
 * Use `before` — a compound cursor `{ createdAt, id }` taken from the **last**
 * (oldest) row of the previous page — to load the next older page. The
 * predicate on the Rust side mirrors `ORDER BY created_at DESC, id DESC`
 * exactly, so same-second siblings are never skipped at a page boundary.
 * A page shorter than `limit` signals the archive is exhausted.
 *
 * Pass `kinds: null` (or omit) to admit all archived kinds. An empty array
 * `[]` matches nothing — callers that want all events must omit/null `kinds`.
 */
export async function readArchivedEvents(
  scopeType: ScopeType,
  scopeValue: string,
  opts?: {
    kinds?: number[] | null;
    before?: { createdAt: number; id: string } | null;
    limit?: number;
  },
): Promise<import("@/shared/api/types").RelayEvent[]> {
  const rawRows = await invokeTauri<string[]>("read_archived_events", {
    scopeType,
    scopeValue,
    kinds: opts?.kinds ?? null,
    beforeCreatedAt: opts?.before?.createdAt ?? null,
    beforeId: opts?.before?.id ?? null,
    limit: opts?.limit ?? null,
  });
  return rawRows
    .map((raw) => {
      try {
        return JSON.parse(raw) as import("@/shared/api/types").RelayEvent;
      } catch {
        console.warn("[tauriArchive] failed to parse archived raw_json:", raw);
        return null;
      }
    })
    .filter((e): e is import("@/shared/api/types").RelayEvent => e !== null);
}
