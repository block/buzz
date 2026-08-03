import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_HUDDLE_ENDED,
  KIND_HUDDLE_PARTICIPANT_JOINED,
  KIND_HUDDLE_PARTICIPANT_LEFT,
  KIND_HUDDLE_STARTED,
} from "@/shared/constants/kinds";
import { HUDDLE_JOINABLE_WINDOW_SECONDS } from "./huddleCardState";

export type HuddleLifecycleState = {
  ended: boolean;
  participants: Set<string>;
  startCreatedAt: number | null;
  staleDeadlineMs: number | null;
};

type ReconstructHuddleOptions = {
  historyMayBeTruncated?: boolean;
  isCurrentHuddle?: boolean;
  nowMs?: number;
  replayComplete?: boolean;
  replayInProgress?: boolean;
};

type SelectActiveHuddleOptions = {
  activeEphemeralChannelId?: string | null;
  historyMayBeTruncated?: boolean;
  historyMayBeTruncatedForEvents?: (events: readonly RelayEvent[]) => boolean;
  nowMs?: number;
  replayComplete?: boolean;
  replayInProgress?: boolean;
  suppressedEphemeralChannelId?: string | null;
};

type IdleHuddleTransitionInput = {
  activeEphemeralChannelId?: string | null;
  displayedEphemeralChannelId?: string | null;
  eventEphemeralChannelId?: string | null;
  lastActiveEphemeralChannelId?: string | null;
};

export type IdleHuddleTransition = {
  shouldClearDisplayedHuddle: boolean;
  suppressedEphemeralChannelId: string | null;
};

export function huddleEventClearsSuppression(event: RelayEvent): boolean {
  return (
    event.kind === KIND_HUDDLE_STARTED ||
    event.kind === KIND_HUDDLE_PARTICIPANT_JOINED ||
    event.kind === KIND_HUDDLE_ENDED
  );
}

export function huddleEventClearsSuppressionForState(
  event: RelayEvent,
  state: HuddleLifecycleState,
): boolean {
  return (
    huddleEventClearsSuppression(event) ||
    (event.kind === KIND_HUDDLE_PARTICIPANT_LEFT &&
      !state.ended &&
      state.participants.size > 0)
  );
}

export const HUDDLE_EVENT_HISTORY_LIMIT = 100;

export type ActiveHuddleLifecycleState = {
  ephemeralChannelId: string;
  state: HuddleLifecycleState;
};

export type HuddleReplayTracker = {
  historyMayBeTruncated: () => boolean;
  historyMayBeTruncatedForEvents: (events: Iterable<RelayEvent>) => boolean;
  markReplayComplete: () => void;
  markReplayFailed: () => void;
  markReplayStarted: (retainedEvents?: Iterable<RelayEvent>) => void;
  replayComplete: () => boolean;
  replayInProgress: () => boolean;
  recordReplayEvent: (event: RelayEvent) => void;
};

const MAX_SET_TIMEOUT_DELAY_MS = 2_147_483_647;

export function createHuddleReplayTracker(
  limit = HUDDLE_EVENT_HISTORY_LIMIT,
): HuddleReplayTracker {
  let historyWasTruncated = false;
  let replayComplete = false;
  let replayInProgress = true;
  let countingReplay = true;
  let replayEventCount = 0;
  let seenReplayEventIds = new Set<string>();
  let preReplayHuddleIds = new Set<string>();
  const truncatedReplayEventIds = new Set<string>();
  const truncatedPreReplayHuddleIds = new Set<string>();

  function currentReplayWasTruncated(): boolean {
    return countingReplay && replayEventCount >= limit;
  }

  function preserveCurrentReplayTruncation() {
    if (!currentReplayWasTruncated()) return;
    for (const eventId of seenReplayEventIds) {
      truncatedReplayEventIds.add(eventId);
    }
    for (const huddleId of preReplayHuddleIds) {
      truncatedPreReplayHuddleIds.add(huddleId);
    }
  }

  return {
    historyMayBeTruncated: () =>
      historyWasTruncated || currentReplayWasTruncated(),
    historyMayBeTruncatedForEvents: (events) => {
      const retainedEvents = [...events];
      return retainedEvents.some((event) => {
        const huddleId = huddleEventChannelId(event);
        const huddleMayBeTruncated =
          huddleId !== null &&
          (truncatedPreReplayHuddleIds.has(huddleId) ||
            (currentReplayWasTruncated() && preReplayHuddleIds.has(huddleId)));
        return (
          truncatedReplayEventIds.has(event.id) ||
          huddleMayBeTruncated ||
          (currentReplayWasTruncated() && seenReplayEventIds.has(event.id))
        );
      });
    },
    markReplayComplete: () => {
      preserveCurrentReplayTruncation();
      replayComplete = true;
      replayInProgress = false;
      countingReplay = false;
      seenReplayEventIds = new Set();
      preReplayHuddleIds = new Set();
    },
    markReplayFailed: () => {
      preserveCurrentReplayTruncation();
      replayComplete = false;
      replayInProgress = false;
      countingReplay = false;
      seenReplayEventIds = new Set();
      preReplayHuddleIds = new Set();
    },
    markReplayStarted: (retainedEvents = []) => {
      preserveCurrentReplayTruncation();
      replayComplete = false;
      replayInProgress = true;
      countingReplay = true;
      replayEventCount = 0;
      seenReplayEventIds = new Set();
      preReplayHuddleIds = new Set(
        [...retainedEvents]
          .map((event) => huddleEventChannelId(event))
          .filter((huddleId): huddleId is string => huddleId !== null),
      );
    },
    replayComplete: () => replayComplete,
    replayInProgress: () => replayInProgress,
    recordReplayEvent: (event: RelayEvent) => {
      if (countingReplay && !seenReplayEventIds.has(event.id)) {
        seenReplayEventIds.add(event.id);
        replayEventCount += 1;
        historyWasTruncated ||= currentReplayWasTruncated();
      }
    },
  };
}

export function huddleEventChannelId(event: RelayEvent): string | null {
  try {
    const parsed = JSON.parse(event.content) as {
      ephemeral_channel_id?: unknown;
    };
    return typeof parsed.ephemeral_channel_id === "string"
      ? parsed.ephemeral_channel_id
      : null;
  } catch {
    return null;
  }
}

/**
 * Record one channel-wide subscription event while retaining only the target
 * huddle's events for reconstruction. The channel-wide IDs preserve whether
 * the relay history query reached its limit before per-huddle filtering.
 */
export function recordHuddleSubscriptionEvent(
  seenChannelEventIds: Set<string>,
  seenHuddleEvents: Map<string, RelayEvent>,
  ephemeralChannelId: string,
  event: RelayEvent,
): boolean {
  if (seenChannelEventIds.has(event.id)) return false;
  seenChannelEventIds.add(event.id);
  if (huddleEventChannelId(event) === ephemeralChannelId) {
    seenHuddleEvents.set(event.id, event);
  }
  return true;
}

export function resolveIdleHuddleTransition({
  activeEphemeralChannelId,
  displayedEphemeralChannelId,
  eventEphemeralChannelId,
  lastActiveEphemeralChannelId,
}: IdleHuddleTransitionInput): IdleHuddleTransition {
  const departingEphemeralChannelId =
    eventEphemeralChannelId ??
    activeEphemeralChannelId ??
    lastActiveEphemeralChannelId ??
    null;
  if (
    departingEphemeralChannelId === null ||
    (departingEphemeralChannelId !== activeEphemeralChannelId &&
      departingEphemeralChannelId !== displayedEphemeralChannelId &&
      departingEphemeralChannelId !== lastActiveEphemeralChannelId)
  ) {
    return {
      shouldClearDisplayedHuddle: false,
      suppressedEphemeralChannelId: null,
    };
  }

  return {
    shouldClearDisplayedHuddle:
      departingEphemeralChannelId === displayedEphemeralChannelId,
    suppressedEphemeralChannelId: departingEphemeralChannelId,
  };
}

export function huddleParticipantDisplayCount(
  participants: ReadonlySet<string>,
  options: { isCurrentHuddle?: boolean } = {},
): number {
  if (options.isCurrentHuddle) {
    return Math.max(participants.size, 1);
  }
  return participants.size;
}

function lifecycleParticipant(event: RelayEvent): string | null {
  return (
    event.tags.find(
      (tag) => tag[0] === "p" && typeof tag[1] === "string",
    )?.[1] ??
    event.pubkey ??
    null
  );
}

/**
 * Reconstruct one huddle from its lifecycle events.
 *
 * An inferred huddle with no START event stays non-terminal because the
 * subscription window may have truncated an older participant JOIN.
 */
export function reconstructHuddleState(
  events: Iterable<RelayEvent>,
  ephemeralChannelId: string,
  options: ReconstructHuddleOptions = {},
): HuddleLifecycleState {
  const matchingEvents = [...events].filter(
    (event) => huddleEventChannelId(event) === ephemeralChannelId,
  );
  const startEvent = matchingEvents
    .filter((event) => event.kind === KIND_HUDDLE_STARTED)
    .sort(
      (left, right) =>
        left.created_at - right.created_at || left.id.localeCompare(right.id),
    )
    .at(-1);
  const participantEvents = matchingEvents
    .filter(
      (event) =>
        event.kind === KIND_HUDDLE_PARTICIPANT_JOINED ||
        event.kind === KIND_HUDDLE_PARTICIPANT_LEFT,
    )
    .sort(
      (left, right) =>
        left.created_at - right.created_at ||
        left.kind - right.kind ||
        left.id.localeCompare(right.id),
    );
  const participantCounts = new Map<string, number>();
  const startPubkey = startEvent?.pubkey ?? null;
  if (startPubkey) {
    participantCounts.set(startPubkey, 1);
  }
  const explicitlyEnded = matchingEvents.some(
    (event) => event.kind === KIND_HUDDLE_ENDED,
  );
  const startCreatedAt = startEvent?.created_at ?? null;
  let startSeedPendingPubkey = startPubkey;

  function participantCount(pubkey: string): number {
    return participantCounts.get(pubkey) ?? 0;
  }

  function incrementParticipant(pubkey: string) {
    participantCounts.set(pubkey, participantCount(pubkey) + 1);
  }

  function decrementParticipant(pubkey: string) {
    const count = participantCount(pubkey);
    if (count <= 1) {
      participantCounts.delete(pubkey);
    } else {
      participantCounts.set(pubkey, count - 1);
    }
  }

  // START is client-signed while participant transitions are relay-signed, so
  // their created_at values are not one causal clock. Seed the creator from
  // START, then let the first matching JOIN or LEFT consume that seed so the
  // retained START does not double-count the creator's first relay peer.
  for (const event of participantEvents) {
    switch (event.kind) {
      case KIND_HUDDLE_PARTICIPANT_JOINED: {
        const pubkey = lifecycleParticipant(event);
        if (!pubkey) break;
        if (pubkey === startSeedPendingPubkey) {
          startSeedPendingPubkey = null;
        } else {
          incrementParticipant(pubkey);
        }
        break;
      }
      case KIND_HUDDLE_PARTICIPANT_LEFT: {
        const pubkey = lifecycleParticipant(event);
        if (!pubkey) break;
        if (pubkey === startSeedPendingPubkey) {
          startSeedPendingPubkey = null;
        }
        decrementParticipant(pubkey);
        break;
      }
    }
  }
  const participants = new Set(participantCounts.keys());
  const hasLiveRelayParticipant = participantEvents.some((event) => {
    if (event.kind !== KIND_HUDDLE_PARTICIPANT_JOINED) return false;
    const pubkey = lifecycleParticipant(event);
    return pubkey !== null && participants.has(pubkey);
  });

  // Retained lifecycle evidence can outlive the ephemeral huddle channel if the
  // relay archives the room without a parent END event. Bound stale non-local
  // sessions by the room's START-based TTL window, while replay is still
  // inconclusive until EOSE. A retained live relay JOIN is stronger evidence
  // than the START fallback because relay activity may reflect a still-live
  // room whose TTL was refreshed by ephemeral-channel traffic.
  const staleDeadlineMs =
    startCreatedAt !== null &&
    !hasLiveRelayParticipant &&
    !options.isCurrentHuddle &&
    !explicitlyEnded &&
    options.replayInProgress !== true
      ? (startCreatedAt + HUDDLE_JOINABLE_WINDOW_SECONDS) * 1000 + 1
      : null;
  const nowMs = options.nowMs ?? Date.now();
  const drainedAfterCompleteReplay =
    options.replayComplete === true &&
    options.historyMayBeTruncated !== true &&
    !options.isCurrentHuddle &&
    startCreatedAt !== null &&
    participantEvents.length > 0 &&
    participants.size === 0;

  return {
    ended:
      explicitlyEnded ||
      drainedAfterCompleteReplay ||
      (staleDeadlineMs !== null && nowMs >= staleDeadlineMs),
    participants,
    startCreatedAt,
    staleDeadlineMs,
  };
}

/**
 * Select the channel header's huddle without falling back past a newer ended
 * session. Retained START events are the session boundaries; participant and
 * END timestamps never compete with a different client's START timestamp.
 */
export function selectActiveHuddleState(
  events: Iterable<RelayEvent>,
  options: SelectActiveHuddleOptions = {},
): ActiveHuddleLifecycleState | null {
  const allEvents = [...events];
  const nowMs = options.nowMs ?? Date.now();
  const nowSeconds = nowMs / 1000;
  const historyMayBeTruncated =
    options.historyMayBeTruncated ??
    allEvents.length >= HUDDLE_EVENT_HISTORY_LIMIT;
  const eventsByHuddle = new Map<string, RelayEvent[]>();
  for (const event of allEvents) {
    const ephemeralChannelId = huddleEventChannelId(event);
    if (!ephemeralChannelId) continue;
    const huddleEvents = eventsByHuddle.get(ephemeralChannelId) ?? [];
    huddleEvents.push(event);
    eventsByHuddle.set(ephemeralChannelId, huddleEvents);
  }

  const candidates = [...eventsByHuddle.entries()].map(
    ([ephemeralChannelId, huddleEvents]) => {
      const relayParticipantEvents = huddleEvents.filter(
        (event) =>
          event.kind === KIND_HUDDLE_PARTICIPANT_JOINED ||
          event.kind === KIND_HUDDLE_PARTICIPANT_LEFT,
      );
      const relayJoinEvents = relayParticipantEvents.filter(
        (event) => event.kind === KIND_HUDDLE_PARTICIPANT_JOINED,
      );
      const latestStartEvent = huddleEvents
        .filter((event) => event.kind === KIND_HUDDLE_STARTED)
        .sort(
          (left, right) =>
            left.created_at - right.created_at ||
            left.id.localeCompare(right.id),
        )
        .at(-1);
      const huddleHistoryMayBeTruncated =
        options.historyMayBeTruncatedForEvents?.(huddleEvents) ??
        historyMayBeTruncated;
      const state = reconstructHuddleState(huddleEvents, ephemeralChannelId, {
        historyMayBeTruncated: huddleHistoryMayBeTruncated,
        isCurrentHuddle:
          options.activeEphemeralChannelId === ephemeralChannelId,
        nowMs,
        replayComplete: options.replayComplete,
        replayInProgress: options.replayInProgress,
      });
      const hasPresentRetainedCreator =
        huddleHistoryMayBeTruncated &&
        latestStartEvent?.pubkey !== undefined &&
        state.participants.has(latestStartEvent.pubkey);
      return {
        ephemeralChannelId,
        state,
        hasRelayParticipantEvent: relayParticipantEvents.length > 0,
        hasPresentRelayParticipant:
          !state.ended &&
          (hasPresentRetainedCreator ||
            relayJoinEvents.some((event) =>
              state.participants.has(lifecycleParticipant(event) ?? ""),
            )),
        historyMayBeTruncated: huddleHistoryMayBeTruncated,
        latestRelayJoinCreatedAt:
          relayJoinEvents.length > 0
            ? Math.max(...relayJoinEvents.map((event) => event.created_at))
            : null,
      };
    },
  );

  const current = candidates.find(
    ({ ephemeralChannelId, state }) =>
      ephemeralChannelId === options.activeEphemeralChannelId && !state.ended,
  );
  if (current) {
    return {
      ephemeralChannelId: current.ephemeralChannelId,
      state: current.state,
    };
  }

  // Relay-signed JOIN events share one clock across huddles, so only the newest
  // relay-backed session may be shown. LEFT is a departure transition within a
  // session, while END is client-emitted room-local evidence; neither may make
  // an older room outrank a newer session. If the newest relay-backed session
  // is terminal, do not resurrect an older relay-backed session. A currently
  // present participant gives that newest relay-backed session priority over
  // every START-only candidate without comparing relay and client clocks.
  const replayInProgress =
    options.replayInProgress ?? options.replayComplete === false;
  const newestRelayCandidate = candidates
    .filter(({ hasRelayParticipantEvent }) => hasRelayParticipantEvent)
    .sort(
      (left, right) =>
        (right.latestRelayJoinCreatedAt ?? 0) -
          (left.latestRelayJoinCreatedAt ?? 0) ||
        Number(right.hasPresentRelayParticipant) -
          Number(left.hasPresentRelayParticipant) ||
        right.ephemeralChannelId.localeCompare(left.ephemeralChannelId),
    )[0];
  const newestRelayStartCreatedAt =
    newestRelayCandidate?.state.startCreatedAt ?? null;
  // Future-skewed client STARTs may keep their own room joinable, but they
  // cannot be used as a terminal barrier over relay-ordered live evidence.
  const newestSuppressedStartCreatedAt =
    candidates
      .filter(
        ({ ephemeralChannelId, state }) =>
          ephemeralChannelId === options.suppressedEphemeralChannelId &&
          state.startCreatedAt !== null &&
          state.startCreatedAt <= nowSeconds,
      )
      .sort(
        (left, right) =>
          (right.state.startCreatedAt ?? 0) -
            (left.state.startCreatedAt ?? 0) ||
          right.ephemeralChannelId.localeCompare(left.ephemeralChannelId),
      )[0]?.state.startCreatedAt ?? null;
  const newestEndedStartCreatedAt =
    candidates
      .filter(
        ({ state }) =>
          state.ended &&
          state.startCreatedAt !== null &&
          state.startCreatedAt <= nowSeconds,
      )
      .sort(
        (left, right) =>
          (right.state.startCreatedAt ?? 0) -
            (left.state.startCreatedAt ?? 0) ||
          right.ephemeralChannelId.localeCompare(left.ephemeralChannelId),
      )[0]?.state.startCreatedAt ?? null;
  const newestTerminalStartBarrierCreatedAt = Math.max(
    newestSuppressedStartCreatedAt ?? Number.NEGATIVE_INFINITY,
    newestEndedStartCreatedAt ?? Number.NEGATIVE_INFINITY,
  );
  if (newestRelayCandidate?.hasPresentRelayParticipant) {
    if (
      newestRelayCandidate.ephemeralChannelId ===
        options.suppressedEphemeralChannelId ||
      (newestRelayStartCreatedAt !== null &&
        newestTerminalStartBarrierCreatedAt > newestRelayStartCreatedAt)
    ) {
      return null;
    }
    return {
      ephemeralChannelId: newestRelayCandidate.ephemeralChannelId,
      state: newestRelayCandidate.state,
    };
  }
  // LEFT-only inconclusive candidates have no JOIN timestamp to sort on. If
  // their retained START boundary is newer than the newest JOIN-backed relay
  // candidate, keep that session instead of falling through to the start action
  // or an older empty relay huddle.
  const newestInconclusiveRelayCandidate = candidates
    .filter(
      ({
        hasPresentRelayParticipant,
        hasRelayParticipantEvent,
        historyMayBeTruncated,
        state,
      }) =>
        hasRelayParticipantEvent &&
        (historyMayBeTruncated || replayInProgress) &&
        !hasPresentRelayParticipant &&
        !state.ended &&
        state.startCreatedAt !== null &&
        (newestRelayStartCreatedAt === null ||
          state.startCreatedAt >= newestRelayStartCreatedAt),
    )
    .sort(
      (left, right) =>
        (right.state.startCreatedAt ?? 0) - (left.state.startCreatedAt ?? 0) ||
        right.ephemeralChannelId.localeCompare(left.ephemeralChannelId),
    )[0];
  if (newestInconclusiveRelayCandidate) {
    if (
      newestInconclusiveRelayCandidate.ephemeralChannelId ===
      options.suppressedEphemeralChannelId
    ) {
      return null;
    }
    return {
      ephemeralChannelId: newestInconclusiveRelayCandidate.ephemeralChannelId,
      state: newestInconclusiveRelayCandidate.state,
    };
  }
  // A truncated or in-flight empty roster is inconclusive: the surviving
  // participant's JOIN can be outside the retained window or not delivered yet.
  // Keep the newest non-terminal relay-backed candidate instead of exposing the
  // start action for a room that may still be live.
  if (
    newestRelayCandidate !== undefined &&
    (newestRelayCandidate.historyMayBeTruncated || replayInProgress) &&
    !newestRelayCandidate.state.ended
  ) {
    if (
      newestRelayCandidate.ephemeralChannelId ===
      options.suppressedEphemeralChannelId
    ) {
      return null;
    }
    return {
      ephemeralChannelId: newestRelayCandidate.ephemeralChannelId,
      state: newestRelayCandidate.state,
    };
  }
  const newestRelayJoinCreatedAt =
    newestRelayCandidate?.latestRelayJoinCreatedAt ?? null;
  const newestStartBarrierCreatedAt = Math.max(
    newestRelayStartCreatedAt ?? Number.NEGATIVE_INFINITY,
    newestTerminalStartBarrierCreatedAt,
  );

  // A START-only session has no relay-clock evidence, so compare it only to a
  // retained START boundary from the newest relay-backed session. A terminal
  // relay session with no retained START cannot prove its relative order
  // against a START-only candidate authored on a different client clock.
  const selected = candidates
    .filter(
      ({ ephemeralChannelId, latestRelayJoinCreatedAt, state }) =>
        ephemeralChannelId !== options.suppressedEphemeralChannelId &&
        latestRelayJoinCreatedAt === null &&
        state.startCreatedAt !== null &&
        (newestRelayJoinCreatedAt === null ||
          newestRelayStartCreatedAt === null ||
          state.startCreatedAt > newestRelayStartCreatedAt) &&
        (newestStartBarrierCreatedAt === Number.NEGATIVE_INFINITY ||
          ephemeralChannelId === newestRelayCandidate?.ephemeralChannelId ||
          state.startCreatedAt > newestStartBarrierCreatedAt) &&
        !state.ended,
    )
    .sort(
      (left, right) =>
        (right.state.startCreatedAt ?? 0) - (left.state.startCreatedAt ?? 0) ||
        right.ephemeralChannelId.localeCompare(left.ephemeralChannelId),
    )[0];

  if (!selected) return null;
  return {
    ephemeralChannelId: selected.ephemeralChannelId,
    state: selected.state,
  };
}

/** Delay until an unconfirmed START crosses the shared stale boundary. */
export function huddleStalenessDelayMs(
  staleDeadlineMs: number | null,
  nowMs = Date.now(),
): number | null {
  if (staleDeadlineMs === null) return null;
  return Math.min(
    Math.max(0, staleDeadlineMs - nowMs),
    MAX_SET_TIMEOUT_DELAY_MS,
  );
}
