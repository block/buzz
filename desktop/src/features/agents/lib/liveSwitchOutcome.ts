import type { ControlResultFrame } from "@/shared/api/types";

const SIGNED_RELAY_EVENT_ID = /^[0-9a-f]{64}$/i;
const CONTROL_REQUEST_ID = /^[0-9a-f]{32}$/;
// Exact unpredictable request correlation is the causal boundary. These
// timestamps are only broad sanity checks: tolerate independently skewed node
// clocks while rejecting old retained events and implausibly future results.
const CONTROL_RESULT_MAX_AGE_MS = 5 * 60_000;
const CONTROL_RESULT_MAX_FUTURE_SKEW_MS = 2 * 60_000;
const CONTROL_RESULT_REPLAY_RETENTION_MS =
  CONTROL_RESULT_MAX_AGE_MS + CONTROL_RESULT_MAX_FUTURE_SKEW_MS + 1;

/**
 * Build a freshness-expiring terminal-proof claim journal.
 *
 * Claims live for the whole interval in which the same signed result could
 * pass the broad freshness check. After that horizon the result itself is too
 * old to satisfy a request, so retaining its ID would only create an unbounded
 * process-lifetime leak. There is deliberately no cardinality cap: a busy
 * desktop must not permanently disable live switching after N operations.
 */
export function createTerminalClaimJournal({
  retentionMs = CONTROL_RESULT_REPLAY_RETENTION_MS,
  now = Date.now,
}: {
  retentionMs?: number;
  now?: () => number;
} = {}): (frame: ControlResultFrame, observedAtMs?: number) => boolean {
  if (!Number.isFinite(retentionMs) || retentionMs <= 0) {
    throw new Error("terminal claim retention must be positive");
  }
  const claimedPairs = new Map<string, number>();
  const expirations: Array<{
    key: string;
    expiresAtMs: number;
  }> = [];
  let expirationCursor = 0;

  const pruneExpired = (observedAtMs: number) => {
    while (
      expirationCursor < expirations.length &&
      expirations[expirationCursor].expiresAtMs <= observedAtMs
    ) {
      const expired = expirations[expirationCursor];
      if (claimedPairs.get(expired.key) === expired.expiresAtMs) {
        claimedPairs.delete(expired.key);
      }
      expirationCursor += 1;
    }
    if (
      expirationCursor >= 1_024 &&
      expirationCursor * 2 >= expirations.length
    ) {
      expirations.splice(0, expirationCursor);
      expirationCursor = 0;
    }
  };

  return (frame, observedAtMs = now()) => {
    const eventId = frame.relayEventId;
    const requestId = frame.requestId;
    if (
      !eventId ||
      !SIGNED_RELAY_EVENT_ID.test(eventId) ||
      !requestId ||
      !CONTROL_REQUEST_ID.test(requestId)
    ) {
      return false;
    }
    if (!Number.isFinite(observedAtMs)) {
      return false;
    }
    pruneExpired(observedAtMs);
    const key = `${eventId}:${requestId}`;
    if (claimedPairs.has(key)) {
      return false;
    }
    const expiresAtMs = observedAtMs + retentionMs;
    claimedPairs.set(key, expiresAtMs);
    expirations.push({ key, expiresAtMs });
    return true;
  };
}

const claimControlResult = createTerminalClaimJournal();

function createControlRequestId(): string {
  const bytes = globalThis.crypto.getRandomValues(new Uint8Array(16));
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join(
    "",
  );
}

function isFreshControlResult(
  frame: ControlResultFrame,
  observedAtMs: number,
): boolean {
  if (
    !frame.relayEventId ||
    !SIGNED_RELAY_EVENT_ID.test(frame.relayEventId) ||
    typeof frame.relayCreatedAt !== "number" ||
    !Number.isSafeInteger(frame.relayCreatedAt) ||
    !frame.observerTimestamp
  ) {
    return false;
  }

  const observerTimestampMs = Date.parse(frame.observerTimestamp);
  if (!Number.isFinite(observerTimestampMs) || !Number.isFinite(observedAtMs)) {
    return false;
  }

  const relayCreatedAtMs = frame.relayCreatedAt * 1_000;
  const oldestAllowedMs = observedAtMs - CONTROL_RESULT_MAX_AGE_MS;
  const newestAllowedMs = observedAtMs + CONTROL_RESULT_MAX_FUTURE_SKEW_MS;
  return (
    relayCreatedAtMs >= oldestAllowedMs &&
    relayCreatedAtMs <= newestAllowedMs &&
    observerTimestampMs >= oldestAllowedMs &&
    observerTimestampMs <= newestAllowedMs
  );
}

/**
 * Resolve the outcome of a live `switch_model` across one or more channels.
 *
 * A live switch fires a `switch_model` frame per active channel and learns each
 * channel's result asynchronously over the observer relay. Receipt (`sent`) and
 * replacement scheduling (`recycling`) are non-terminal. Success requires a
 * distinct `switched` application proof from every target channel; repeated
 * frames from one channel cannot satisfy another. Unsupported and explicit
 * failure statuses fail fast. A timeout is reported as `pending`, never as
 * success.
 *
 * The counting lives here, isolated from React and the relay so it can be unit
 * tested with synthetic frames and a fake clock. The caller injects the
 * relay subscription, the per-channel sends, and the timeout scheduler.
 */
export async function awaitLiveSwitchOutcome({
  channelIds,
  modelId,
  createRequestId = createControlRequestId,
  subscribe,
  sendSwitches,
  scheduleTimeout,
  now = Date.now,
}: {
  /** Exact channels the switch was fired to. */
  channelIds: string[];
  /** Model being switched to; frames for any other model are ignored. */
  modelId: string;
  /** Test seam for the unique control request identifier. */
  createRequestId?: () => string;
  /** Register a control-result listener; returns an unsubscribe function. */
  subscribe: (listener: (frame: ControlResultFrame) => void) => () => void;
  /** Fire the per-channel sends with this operation's exact request ID. */
  sendSwitches: (requestId: string) => Promise<void>;
  /** Schedule the no-reply fallback; returns a cancel function. */
  scheduleTimeout: (onTimeout: () => void) => () => void;
  /** Clock seam for receive-time freshness checks and replay-claim expiry. */
  now?: () => number;
}): Promise<"ok" | "unsupported" | "failed" | "pending"> {
  const requestId = createRequestId();
  if (!CONTROL_REQUEST_ID.test(requestId)) {
    throw new Error(
      "live model-switch request ID must be 32 lowercase hex characters",
    );
  }
  const targetChannels = new Set(channelIds);
  if (targetChannels.size === 0) {
    await sendSwitches(requestId);
    return "ok";
  }

  let unsubscribe = () => {};
  let cancelTimeout = () => {};
  let finished = false;
  const appliedChannels = new Set<string>();
  let finish: (outcome: "ok" | "unsupported" | "failed" | "pending") => void =
    () => {};
  const settled = new Promise<"ok" | "unsupported" | "failed" | "pending">(
    (resolve) => {
      finish = (outcome) => {
        if (finished) return;
        finished = true;
        cancelTimeout();
        unsubscribe();
        resolve(outcome);
      };
      cancelTimeout = scheduleTimeout(() => finish("pending"));
      unsubscribe = subscribe((frame) => {
        const observedAtMs = now();
        if (
          frame.type !== "switch_model" ||
          frame.requestId !== requestId ||
          frame.modelId !== modelId ||
          !frame.channelId ||
          !targetChannels.has(frame.channelId) ||
          !isFreshControlResult(frame, observedAtMs)
        ) {
          return;
        }
        const isFailure =
          frame.status === "unsupported_model" ||
          frame.status === "switch_failed" ||
          frame.status === "turn_ending" ||
          frame.status === "no_active_turn";
        const isNewApplication =
          frame.status === "switched" && !appliedChannels.has(frame.channelId);
        if (
          (isFailure || isNewApplication) &&
          !claimControlResult(frame, observedAtMs)
        ) {
          return;
        }
        if (frame.status === "unsupported_model") {
          finish("unsupported");
          return;
        }
        if (
          frame.status === "switch_failed" ||
          frame.status === "turn_ending" ||
          frame.status === "no_active_turn"
        ) {
          finish("failed");
          return;
        }
        if (frame.status !== "switched") {
          // `sent`, `recycling`, and unknown future non-terminal statuses do not
          // prove that the model reached this channel's ACP session.
          return;
        }
        appliedChannels.add(frame.channelId);
        if (appliedChannels.size === targetChannels.size) {
          finish("ok");
        }
      });
    },
  );

  let sends: Promise<void>;
  try {
    sends = sendSwitches(requestId);
  } catch (error) {
    finished = true;
    cancelTimeout();
    unsubscribe();
    throw error;
  }
  const sendFailure = sends.then<never>(
    () => new Promise<never>(() => {}),
    (error) => {
      if (!finished) {
        finished = true;
        cancelTimeout();
        unsubscribe();
      }
      throw error;
    },
  );

  return Promise.race([settled, sendFailure]);
}
