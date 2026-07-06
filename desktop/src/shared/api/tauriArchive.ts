import { invokeTauri } from "./tauri";

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
  dropped: number;
};

// ── Subscription-change notifier ─────────────────────────────────────────────

/**
 * Module-level notifier for subscription mutations (create/delete).
 * The archive sync manager subscribes to this to reload live subscriptions
 * without needing manager instances threaded through UI props.
 */
const subscriptionChangeListeners = new Set<() => void>();

export function onSubscriptionChange(listener: () => void): () => void {
  subscriptionChangeListeners.add(listener);
  return () => subscriptionChangeListeners.delete(listener);
}

function notifySubscriptionChange(): void {
  for (const listener of subscriptionChangeListeners) {
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
 * Returns `true` when the build has observer-feed archive default-on.
 *
 * Internal builds set `BUZZ_BUILD_OBSERVER_ARCHIVE_DEFAULT` at build time;
 * OSS builds never set it, so this returns `false`.  The frontend calls this
 * once at startup to decide whether to auto-seed an `owner_p` subscription.
 */
export async function observerArchiveDefaultEnabled(): Promise<boolean> {
  return invokeTauri<boolean>("observer_archive_default_enabled");
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
  notifySubscriptionChange();
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
  if (removed) {
    notifySubscriptionChange();
  }
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
  return invokeTauri<ArchiveBatchResult>("archive_events", {
    candidates: candidates.map((c) => ({
      raw_event_json: c.rawEventJson,
      matched_scope: {
        scope_type: c.matchedScope.scopeType,
        scope_value: c.matchedScope.scopeValue,
      },
    })),
  });
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
