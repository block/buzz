import type { RelayEvent } from "@/shared/api/types";

/**
 * Observable connection state for the relay singleton.
 *
 * - `idle`         — never tried to connect yet (post-init, pre-community).
 * - `connecting`   — initial socket + AUTH handshake in flight.
 * - `connected`    — socket open and AUTH'd.
 * - `reconnecting` — socket dropped, waiting for the backoff timer.
 * - `stalled`      — socket is *open* per the WS layer but no inbound frames
 *                    for a long time (half-open socket / VPN split-brain). We
 *                    surface this so the UI can warn even though tungstenite
 *                    hasn't reported anything wrong yet.
 * - `disconnected` — final/terminal disconnect (auth rejected, community
 *                    switch, etc.) — no auto-reconnect scheduled.
 */
export type ConnectionState =
  | "idle"
  | "connecting"
  | "connected"
  | "reconnecting"
  | "stalled"
  | "disconnected";

/** True when the UI should surface a "connection lost" indicator. */
export function isRelayConnectionDegraded(state: ConnectionState): boolean {
  return (
    state === "reconnecting" || state === "stalled" || state === "disconnected"
  );
}

export type RelaySubscriptionFilter = {
  ids?: string[];
  kinds: number[];
  limit: number;
  authors?: string[];
  since?: number;
  until?: number;
} & Partial<Record<`#${string}`, string[]>>;

type HistorySubscription = {
  mode: "history";
  events: RelayEvent[];
  resolve: (events: RelayEvent[]) => void;
  reject: (error: Error) => void;
  timeout: number;
};

type FirstEventSubscription = {
  mode: "first";
  onEvent: (event: RelayEvent) => void;
  resolve: (event: RelayEvent | null) => void;
  reject: (error: Error) => void;
  timeout: number;
};

type LiveSubscription = {
  mode: "live";
  filter: RelaySubscriptionFilter;
  onEvent: (event: RelayEvent) => void;
  resolveReady?: () => void;
  lastSeenCreatedAt?: number;
  /**
   * Lower bound of a reconnect backfill window that has not yet completed.
   *
   * Events on the restored live REQ advance `lastSeenCreatedAt` regardless of
   * backfill success, so after an exhausted backfill the cursor alone would
   * make the next reconnect skip the unresolved older window — silent message
   * loss. This floor is pinned when paging starts and cleared only when a
   * backfill pass completes; the next replay starts from
   * `min(pendingReplaySince, cursor window)`.
   */
  pendingReplaySince?: number;
  closedRetryAttempt?: number;
  closedRetryTimeout?: number;
};

/**
 * Fenced subscription for NIP-RS full-state loads.
 *
 * Unlike `live`, a fenced subscription:
 * - Resolves `established` ONLY on this subscription's own EOSE — never via
 *   a fallback timer or terminal CLOSED.
 * - Delivers events synchronously (bypasses the EVENT_BATCH_MS buffer) so
 *   the loader's synchronous drain barrier needs no timer.
 * - On CLOSED or any connection lapse, sets `lapsed = true` and resolves
 *   `established` (in case the loader is still awaiting it), then removes
 *   itself without retrying.
 */
export type FencedSubscription = {
  mode: "fenced";
  filter: RelaySubscriptionFilter;
  onEvent: (event: RelayEvent) => void;
  /** Connection generation at subscribe time — mismatch = lapsed. */
  generation: number;
  /** Resolves on EOSE or lapse (check `lapsed` after awaiting). */
  resolveEstablished: () => void;
  /** True after CLOSED, reconnect, or generation change. */
  lapsed: boolean;
};

/**
 * Handle returned by `RelayClient.subscribeFenced()`.
 *
 * - `established` resolves ONLY on the subscription's own EOSE, or resolves
 *   after setting `lapsed = true` if the connection lapses before EOSE.
 * - `lapsed` is synchronously readable — check it after awaiting `established`
 *   and at any point during enumeration.
 * - `unsubscribe()` closes the relay subscription when the load is complete.
 */
export type FenceHandle = {
  /** Resolves on EOSE (check `lapsed` after awaiting). */
  established: Promise<void>;
  /** True if the connection lapsed before or after EOSE. */
  readonly lapsed: boolean;
  /** Close the relay subscription. */
  unsubscribe(): Promise<void>;
};

export function buildFenceHandle(
  sub: FencedSubscription,
  established: Promise<void>,
  unsubscribe: () => Promise<void>,
): FenceHandle {
  return {
    established,
    get lapsed() {
      return sub.lapsed;
    },
    unsubscribe,
  };
}

/**
 * Millis to wait for EOSE after the REQ is sent.  If the relay keeps the
 * socket alive but never sends this subscription's EOSE, the fence lapses and
 * the load concludes `complete: false`.  Reconnect / `retryLoad` owns recovery.
 * Must NEVER count as establishment.
 */
export const FENCED_ESTABLISHMENT_TIMEOUT_MS = 30_000;

/**
 * Core of `RelayClient.subscribeFenced()` extracted for testability.
 *
 * Registers a fenced subscription, sends REQ, and returns a `FenceHandle`
 * whose `established` resolves ONLY on EOSE — never on a fallback timer or
 * CLOSED. Any lapse (generation mismatch, send failure, `resetConnection`,
 * or establishment timeout) sets `lapsed = true` and resolves `established`
 * so no caller suspends forever.
 */
export async function createFencedSubscription(
  deps: {
    connectionGeneration: () => number;
    subscriptions: Map<string, RelaySubscription>;
    sendReq: (subId: string, filter: RelaySubscriptionFilter) => Promise<void>;
    closeSub: (subId: string) => Promise<void>;
    /** Override the establishment timeout (ms). Defaults to FENCED_ESTABLISHMENT_TIMEOUT_MS. */
    establishmentTimeoutMs?: number;
  },
  filter: RelaySubscriptionFilter,
  onEvent: (event: RelayEvent) => void,
): Promise<FenceHandle> {
  const generation = deps.connectionGeneration();
  let resolveEstablished = () => {};
  const established = new Promise<void>((r) => {
    resolveEstablished = r;
  });
  const subId = `fenced-${crypto.randomUUID()}`;
  const fencedSub: FencedSubscription = {
    mode: "fenced",
    filter,
    onEvent,
    generation,
    resolveEstablished,
    lapsed: false,
  };
  const lapseAndReturn = (): FenceHandle => {
    fencedSub.lapsed = true;
    resolveEstablished();
    deps.subscriptions.delete(subId);
    return buildFenceHandle(fencedSub, established, async () => {});
  };
  if (deps.connectionGeneration() !== generation) return lapseAndReturn();
  deps.subscriptions.set(subId, fencedSub);

  // ── Establishment timeout ───────────────────────────────────────────────
  // Install the timeout and wrapped resolver BEFORE sending REQ so that EOSE
  // delivered synchronously during sendReq() (e.g. from a fake or fast relay)
  // cancels the timer correctly and never lapses an already-established fence.
  //
  // A relay that stays alive but never delivers this subscription's EOSE must
  // not hang initialize() forever.  The timeout lapses the fence exactly like
  // a CLOSED or reconnect — it NEVER counts as establishment.
  const timeoutMs =
    deps.establishmentTimeoutMs ?? FENCED_ESTABLISHMENT_TIMEOUT_MS;
  let establishmentTimer: ReturnType<typeof setTimeout> | null = null;
  let alreadyEstablished = false;

  // Wrap resolveEstablished so the timer is cancelled on normal EOSE.
  const originalResolve = fencedSub.resolveEstablished;
  fencedSub.resolveEstablished = () => {
    alreadyEstablished = true;
    if (establishmentTimer !== null) {
      clearTimeout(establishmentTimer);
      establishmentTimer = null;
    }
    originalResolve();
  };
  // Also patch the closed reference so the promise resolves via the wrapper.
  resolveEstablished = fencedSub.resolveEstablished;

  establishmentTimer = setTimeout(() => {
    establishmentTimer = null;
    // Only lapse if EOSE has not already established the fence.
    if (
      !alreadyEstablished &&
      !fencedSub.lapsed &&
      deps.subscriptions.get(subId) === fencedSub
    ) {
      fencedSub.lapsed = true;
      fencedSub.resolveEstablished();
      deps.subscriptions.delete(subId);
    }
  }, timeoutMs);

  try {
    await deps.sendReq(subId, filter);
  } catch {
    return lapseAndReturn();
  }
  if (deps.connectionGeneration() !== generation || fencedSub.lapsed)
    return lapseAndReturn();

  return buildFenceHandle(fencedSub, established, async () => {
    if (establishmentTimer !== null) {
      clearTimeout(establishmentTimer);
      establishmentTimer = null;
    }
    if (deps.subscriptions.get(subId) !== fencedSub) return;
    deps.subscriptions.delete(subId);
    await deps.closeSub(subId);
  });
}

export type PendingEvent = {
  event: RelayEvent;
  resolve: (event: RelayEvent) => void;
  reject: (error: Error) => void;
  timeout: number;
};

export type RelaySubscription =
  | HistorySubscription
  | FirstEventSubscription
  | LiveSubscription
  | FencedSubscription;

export function sortEvents(events: RelayEvent[]) {
  return [...events].sort((left, right) => {
    if (left.created_at !== right.created_at) {
      return left.created_at - right.created_at;
    }
    // Same (created_at, id) tiebreak as the cache sort (sortMessages) so a
    // history REQ resolves same-second events in a stable, relay-matching
    // order. Currently every consumer re-sorts downstream, but keeping the
    // two sorts on one invariant avoids a latent ordering drift.
    return left.id < right.id ? -1 : left.id > right.id ? 1 : 0;
  });
}

export function getTextPayload(message: unknown) {
  if (typeof message === "string") {
    return message;
  }

  if (
    typeof message === "object" &&
    message !== null &&
    "type" in message &&
    message.type === "Text" &&
    "data" in message &&
    typeof message.data === "string"
  ) {
    return message.data;
  }

  if (
    typeof message === "object" &&
    message !== null &&
    "Text" in message &&
    typeof message.Text === "string"
  ) {
    return message.Text;
  }

  return null;
}
