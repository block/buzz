import { Channel } from "@tauri-apps/api/core";
import * as React from "react";

export type CurrentProjection = Readonly<{
  eventAuthorPubkey: string;
  freshUntil: number;
  connectionEpoch: string;
}>;

type TimerHandle = ReturnType<typeof setTimeout>;

type CurrentProjectionStoreOptions = {
  nowSeconds?: () => number;
  setTimeout?: (callback: () => void, delayMs: number) => TimerHandle;
  clearTimeout?: (handle: TimerHandle) => void;
  maxTimerDelayMs?: number;
  onListenerError?: () => void;
};

export type CurrentProjectionStore = {
  getSnapshot: () => CurrentProjection | null;
  subscribe: (listener: () => void) => () => void;
  replaceFromNative: (candidate: unknown) => void;
  clear: () => void;
};

const LOWERCASE_HEX_PUBKEY = /^[0-9a-f]{64}$/;
const CURRENT_PROJECTION_KEYS = [
  "connectionEpoch",
  "eventAuthorPubkey",
  "freshUntil",
] as const;
const DEFAULT_MAX_TIMER_DELAY_MS = 2_147_483_647;

function logListenerError(): void {
  // Do not include the exception or current DTO: either could contain native
  // payload data outside the browser projection contract.
  console.error("[currentProjectionStore] subscriber failed");
}

/**
 * Copy the narrow native DTO into a frozen browser-owned value.
 *
 * Unknown properties fail closed. Expired projections are represented by
 * null; the deadline is exclusive.
 */
export function parseCurrentProjection(
  candidate: unknown,
  nowSeconds: number,
): CurrentProjection | null {
  if (
    candidate === null ||
    typeof candidate !== "object" ||
    Array.isArray(candidate) ||
    !Number.isFinite(nowSeconds)
  ) {
    return null;
  }

  const value = candidate as Record<string, unknown>;
  const keys = Object.keys(value).sort();
  if (
    keys.length !== CURRENT_PROJECTION_KEYS.length ||
    !CURRENT_PROJECTION_KEYS.every((key, index) => keys[index] === key)
  ) {
    return null;
  }
  const { eventAuthorPubkey, freshUntil, connectionEpoch } = value;
  if (
    typeof eventAuthorPubkey !== "string" ||
    !LOWERCASE_HEX_PUBKEY.test(eventAuthorPubkey) ||
    typeof freshUntil !== "number" ||
    !Number.isSafeInteger(freshUntil) ||
    freshUntil <= 0 ||
    freshUntil <= nowSeconds ||
    typeof connectionEpoch !== "string" ||
    connectionEpoch.length === 0
  ) {
    return null;
  }

  return Object.freeze({
    eventAuthorPubkey,
    freshUntil,
    connectionEpoch,
  });
}

export function createCurrentProjectionStore(
  options: CurrentProjectionStoreOptions = {},
): CurrentProjectionStore {
  const nowSeconds = options.nowSeconds ?? (() => Date.now() / 1_000);
  const schedule = options.setTimeout ?? globalThis.setTimeout.bind(globalThis);
  const cancel =
    options.clearTimeout ?? globalThis.clearTimeout.bind(globalThis);
  const onListenerError = options.onListenerError ?? logListenerError;
  const configuredMaxDelay = options.maxTimerDelayMs;
  const maxTimerDelayMs =
    typeof configuredMaxDelay === "number" &&
    Number.isFinite(configuredMaxDelay) &&
    configuredMaxDelay >= 1
      ? Math.min(Math.floor(configuredMaxDelay), DEFAULT_MAX_TIMER_DELAY_MS)
      : DEFAULT_MAX_TIMER_DELAY_MS;

  let snapshot: CurrentProjection | null = null;
  let expiryTimer: TimerHandle | null = null;
  let workToken = 0;
  const listeners = new Set<() => void>();

  const emitChange = () => {
    for (const listener of [...listeners]) {
      try {
        listener();
      } catch {
        try {
          onListenerError();
        } catch {
          // Logging is best-effort and must not break the state transition.
        }
      }
    }
  };

  const invalidatePendingWork = (): number => {
    workToken += 1;
    if (expiryTimer !== null) {
      cancel(expiryTimer);
      expiryTimer = null;
    }
    return workToken;
  };

  const clear = () => {
    // Invalidate even when already clear: a late callback must never be able
    // to restore or disturb state after an idempotent boundary reset.
    invalidatePendingWork();
    if (snapshot === null) return;
    snapshot = null;
    emitChange();
  };

  const armExpiry = (
    projection: CurrentProjection,
    capturedToken: number,
  ): boolean => {
    if (capturedToken !== workToken) return false;

    const now = nowSeconds();
    if (!Number.isFinite(now) || now >= projection.freshUntil) {
      return false;
    }

    const remainingSeconds = projection.freshUntil - now;
    const delayMs =
      remainingSeconds >= maxTimerDelayMs / 1_000
        ? maxTimerDelayMs
        : Math.max(1, Math.ceil(remainingSeconds * 1_000));

    let scheduledTimer: TimerHandle;
    try {
      scheduledTimer = schedule(() => {
        if (expiryTimer === scheduledTimer) expiryTimer = null;
        if (capturedToken !== workToken) return;

        // Timers are capped and wall clocks can move backwards. Rearm until
        // the exclusive Unix-seconds boundary has actually been reached.
        const firedAt = nowSeconds();
        if (Number.isFinite(firedAt) && firedAt < projection.freshUntil) {
          if (
            !armExpiry(projection, capturedToken) &&
            capturedToken === workToken
          ) {
            clear();
          }
          return;
        }
        clear();
      }, delayMs);
    } catch {
      return false;
    }
    expiryTimer = scheduledTimer;
    return true;
  };

  return {
    getSnapshot: () => snapshot,
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    replaceFromNative(candidate) {
      const projection = parseCurrentProjection(candidate, nowSeconds());
      const capturedToken = invalidatePendingWork();
      if (projection === null) {
        if (snapshot === null) return;
        snapshot = null;
        emitChange();
        return;
      }

      // Arm passive expiry before making the projection observable. A timer
      // host failure therefore cannot leave a current snapshot without an
      // expiry path.
      if (!armExpiry(projection, capturedToken)) {
        if (capturedToken === workToken) clear();
        return;
      }
      snapshot = projection;
      emitChange();
    },
    clear,
  };
}

const currentProjectionStore = createCurrentProjectionStore();

/**
 * Create the sole non-null intake for the browser-owned projection store.
 * The caller's fence binds delivery to one current native status connection.
 */
export function createCurrentProjectionChannel(
  isCurrentConnection: () => boolean,
): Channel<CurrentProjection | null> {
  return new Channel<CurrentProjection | null>((candidate) => {
    if (!isCurrentConnection()) return;
    currentProjectionStore.replaceFromNative(candidate);
  });
}

export function clearCurrentProjection(): void {
  currentProjectionStore.clear();
}

/** Community-boundary reset for the module-level, memory-only singleton. */
export function resetCurrentProjectionStore(): void {
  currentProjectionStore.clear();
}

export function getCurrentProjectionSnapshot(): CurrentProjection | null {
  return currentProjectionStore.getSnapshot();
}

export function subscribeToCurrentProjection(listener: () => void): () => void {
  return currentProjectionStore.subscribe(listener);
}

export function useCurrentProjection(): CurrentProjection | null {
  return React.useSyncExternalStore(
    subscribeToCurrentProjection,
    getCurrentProjectionSnapshot,
    () => null,
  );
}
