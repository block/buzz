import assert from "node:assert/strict";
import test from "node:test";

import {
  createHuddleReplayTracker,
  huddleEventClearsSuppression,
  huddleEventClearsSuppressionForState,
  huddleParticipantDisplayCount,
  huddleStalenessDelayMs,
  recordHuddleSubscriptionEvent,
  reconstructHuddleState,
  resolveIdleHuddleTransition,
  selectActiveHuddleState,
} from "./huddleLifecycleState.ts";

const HUDDLE_ID = "huddle-id";
const CREATOR = "a".repeat(64);
const PARTICIPANT = "b".repeat(64);
const NOW_SECONDS = 2_000_000;

function lifecycleEvent(kind, overrides = {}) {
  return {
    id: `${kind}-${overrides.created_at ?? NOW_SECONDS}`,
    pubkey: CREATOR,
    created_at: NOW_SECONDS,
    kind,
    tags: [],
    content: JSON.stringify({ ephemeral_channel_id: HUDDLE_ID }),
    sig: "",
    ...overrides,
  };
}

function eventForHuddle(kind, ephemeralChannelId, overrides = {}) {
  return lifecycleEvent(kind, {
    content: JSON.stringify({ ephemeral_channel_id: ephemeralChannelId }),
    ...overrides,
  });
}

test("reconstructHuddleState ends an explicitly ended huddle", () => {
  const state = reconstructHuddleState(
    [lifecycleEvent(48100), lifecycleEvent(48103)],
    HUDDLE_ID,
    { nowMs: NOW_SECONDS * 1000 },
  );

  assert.equal(state.ended, true);
  assert.equal(state.startCreatedAt, NOW_SECONDS);
});

test("reconstructHuddleState folds the participant roster for an ended huddle", () => {
  const state = reconstructHuddleState(
    [
      lifecycleEvent(48100, { created_at: NOW_SECONDS - 4 }),
      lifecycleEvent(48101, {
        created_at: NOW_SECONDS - 3,
        tags: [["p", PARTICIPANT]],
      }),
      lifecycleEvent(48102, {
        created_at: NOW_SECONDS - 2,
        tags: [["p", PARTICIPANT]],
      }),
      lifecycleEvent(48102, {
        created_at: NOW_SECONDS - 1,
        tags: [["p", CREATOR]],
      }),
      lifecycleEvent(48103),
    ],
    HUDDLE_ID,
    { nowMs: NOW_SECONDS * 1000 },
  );

  assert.equal(state.ended, true);
  assert.equal(state.participants.size, 0);
});

test("reconstructHuddleState keeps an in-flight empty roster joinable without END", () => {
  const state = reconstructHuddleState(
    [
      lifecycleEvent(48100, { created_at: NOW_SECONDS - 2 }),
      lifecycleEvent(48102, {
        created_at: NOW_SECONDS - 1,
        tags: [["p", CREATOR]],
      }),
    ],
    HUDDLE_ID,
    { nowMs: NOW_SECONDS * 1000, replayComplete: false },
  );

  assert.equal(state.ended, false);
  assert.equal(state.participants.size, 0);

  const completeState = reconstructHuddleState(
    [
      lifecycleEvent(48100, { created_at: NOW_SECONDS - 2 }),
      lifecycleEvent(48102, {
        created_at: NOW_SECONDS - 1,
        tags: [["p", CREATOR]],
      }),
    ],
    HUDDLE_ID,
    { nowMs: NOW_SECONDS * 1000, replayComplete: true },
  );

  assert.equal(completeState.ended, true);
  assert.equal(completeState.participants.size, 0);
});

test("reconstructHuddleState keeps truncated empty rosters inconclusive after EOSE", () => {
  const state = reconstructHuddleState(
    [
      lifecycleEvent(48100, { created_at: NOW_SECONDS - 2 }),
      lifecycleEvent(48102, {
        created_at: NOW_SECONDS - 1,
        tags: [["p", CREATOR]],
      }),
    ],
    HUDDLE_ID,
    {
      historyMayBeTruncated: true,
      nowMs: NOW_SECONDS * 1000,
      replayComplete: true,
    },
  );

  assert.equal(state.ended, false);
  assert.equal(state.participants.size, 0);
});

test("reconstructHuddleState keeps a fully empty relay roster joinable", () => {
  const state = reconstructHuddleState(
    [
      lifecycleEvent(48100),
      lifecycleEvent(48101, { tags: [["p", PARTICIPANT]] }),
      lifecycleEvent(48102, {
        created_at: NOW_SECONDS + 1,
        tags: [["p", PARTICIPANT]],
      }),
      lifecycleEvent(48102, {
        created_at: NOW_SECONDS + 1,
        tags: [["p", CREATOR]],
      }),
    ],
    HUDDLE_ID,
    { nowMs: (NOW_SECONDS + 1) * 1000 },
  );

  assert.equal(state.ended, false);
  assert.equal(state.participants.size, 0);
});

test("reconstructHuddleState does not double-count the creator START seed", () => {
  const state = reconstructHuddleState(
    [
      lifecycleEvent(48100, { created_at: NOW_SECONDS - 3 }),
      lifecycleEvent(48101, {
        created_at: NOW_SECONDS - 2,
        id: "creator-join",
        tags: [["p", CREATOR]],
      }),
      lifecycleEvent(48102, {
        created_at: NOW_SECONDS - 1,
        id: "creator-left",
        tags: [["p", CREATOR]],
      }),
    ],
    HUDDLE_ID,
    { nowMs: NOW_SECONDS * 1000 },
  );

  assert.equal(state.ended, false);
  assert.equal(state.participants.size, 0);
});

test("reconstructHuddleState preserves same-pubkey peer multiplicity", () => {
  const state = reconstructHuddleState(
    [
      lifecycleEvent(48100, { created_at: NOW_SECONDS - 5 }),
      lifecycleEvent(48102, {
        created_at: NOW_SECONDS - 4,
        id: "creator-left",
        tags: [["p", CREATOR]],
      }),
      lifecycleEvent(48101, {
        created_at: NOW_SECONDS - 3,
        id: "participant-join-one",
        tags: [["p", PARTICIPANT]],
      }),
      lifecycleEvent(48101, {
        created_at: NOW_SECONDS - 2,
        id: "participant-join-two",
        tags: [["p", PARTICIPANT]],
      }),
      lifecycleEvent(48102, {
        created_at: NOW_SECONDS - 1,
        id: "participant-left-one",
        tags: [["p", PARTICIPANT]],
      }),
    ],
    HUDDLE_ID,
    { nowMs: NOW_SECONDS * 1000 },
  );

  assert.equal(state.ended, false);
  assert.deepEqual([...state.participants], [PARTICIPANT]);
});

test("reconstructHuddleState ends a stale huddle and retains its start time", () => {
  const startCreatedAt = NOW_SECONDS - 60 * 60 - 1;
  const state = reconstructHuddleState(
    [lifecycleEvent(48100, { created_at: startCreatedAt })],
    HUDDLE_ID,
    { nowMs: NOW_SECONDS * 1000 },
  );

  assert.equal(state.ended, true);
  assert.equal(state.startCreatedAt, startCreatedAt);
  assert.deepEqual([...state.participants], [CREATOR]);
});

test("reconstructHuddleState ends stale lifecycle evidence after participant events", () => {
  const startCreatedAt = NOW_SECONDS - 60 * 60 - 1;
  const state = reconstructHuddleState(
    [
      lifecycleEvent(48100, { created_at: startCreatedAt }),
      lifecycleEvent(48101, {
        created_at: NOW_SECONDS - 1,
        tags: [["p", PARTICIPANT]],
      }),
      lifecycleEvent(48102, {
        tags: [["p", PARTICIPANT]],
      }),
    ],
    HUDDLE_ID,
    { nowMs: NOW_SECONDS * 1000, replayComplete: true },
  );

  assert.equal(state.ended, true);
  assert.equal(state.staleDeadlineMs, (startCreatedAt + 60 * 60) * 1000 + 1);
  assert.deepEqual([...state.participants], [CREATOR]);
});

test("reconstructHuddleState documents bounded staleness extension under maximum future skew", () => {
  const maxClientClockSkewSeconds = 15 * 60;
  const startCreatedAt = NOW_SECONDS + maxClientClockSkewSeconds;
  const state = reconstructHuddleState(
    [lifecycleEvent(48100, { created_at: startCreatedAt })],
    HUDDLE_ID,
    { nowMs: NOW_SECONDS * 1000 },
  );

  assert.equal(state.ended, false);
  assert.equal(state.staleDeadlineMs, (startCreatedAt + 60 * 60) * 1000 + 1);
});

test("reconstructHuddleState keeps a live relay participant after START staleness", () => {
  const startCreatedAt = NOW_SECONDS - 60 * 60 - 1;
  const state = reconstructHuddleState(
    [
      lifecycleEvent(48100, { created_at: startCreatedAt }),
      lifecycleEvent(48101, { tags: [["p", PARTICIPANT]] }),
    ],
    HUDDLE_ID,
    { nowMs: NOW_SECONDS * 1000 },
  );

  assert.equal(state.ended, false);
  assert.equal(state.staleDeadlineMs, null);
  assert.deepEqual([...state.participants], [CREATOR, PARTICIPANT]);
});

test("reconstructHuddleState preserves a JOIN timestamped before START", () => {
  const startCreatedAt = NOW_SECONDS - 10;
  const state = reconstructHuddleState(
    [
      lifecycleEvent(48100, { created_at: startCreatedAt }),
      lifecycleEvent(48101, {
        created_at: startCreatedAt - 5,
        tags: [["p", PARTICIPANT]],
      }),
    ],
    HUDDLE_ID,
    { nowMs: NOW_SECONDS * 1000 },
  );

  assert.equal(state.ended, false);
  assert.equal(state.staleDeadlineMs, null);
  assert.deepEqual([...state.participants], [CREATOR, PARTICIPANT]);
});

test("reconstructHuddleState applies a LEFT timestamped before START", () => {
  const state = reconstructHuddleState(
    [
      lifecycleEvent(48100),
      lifecycleEvent(48102, {
        created_at: NOW_SECONDS - 5,
        tags: [["p", CREATOR]],
      }),
    ],
    HUDDLE_ID,
    { nowMs: NOW_SECONDS * 1000 },
  );

  assert.equal(state.ended, false);
  assert.equal(state.participants.size, 0);
});

test("reconstructHuddleState keeps an empty relay roster joinable under START clock skew", () => {
  const state = reconstructHuddleState(
    [
      lifecycleEvent(48100),
      lifecycleEvent(48101, {
        created_at: NOW_SECONDS - 5,
        tags: [["p", PARTICIPANT]],
      }),
      lifecycleEvent(48102, {
        created_at: NOW_SECONDS - 4,
        tags: [["p", PARTICIPANT]],
      }),
      lifecycleEvent(48102, {
        created_at: NOW_SECONDS - 3,
        tags: [["p", CREATOR]],
      }),
    ],
    HUDDLE_ID,
    { nowMs: NOW_SECONDS * 1000 },
  );

  assert.equal(state.ended, false);
  assert.equal(state.participants.size, 0);
});

test("reconstructHuddleState keeps the current huddle active past START age", () => {
  const startCreatedAt = NOW_SECONDS - 60 * 60 - 1;
  const state = reconstructHuddleState(
    [lifecycleEvent(48100, { created_at: startCreatedAt })],
    HUDDLE_ID,
    { isCurrentHuddle: true, nowMs: NOW_SECONDS * 1000 },
  );

  assert.equal(state.ended, false);
  assert.equal(state.staleDeadlineMs, null);
  assert.deepEqual([...state.participants], [CREATOR]);
});

test("reconstructHuddleState keeps the current huddle active after a local leave", () => {
  const state = reconstructHuddleState(
    [
      lifecycleEvent(48100, { created_at: NOW_SECONDS - 2 }),
      lifecycleEvent(48102, {
        created_at: NOW_SECONDS - 1,
        tags: [["p", CREATOR]],
      }),
    ],
    HUDDLE_ID,
    {
      isCurrentHuddle: true,
      nowMs: NOW_SECONDS * 1000,
      replayComplete: true,
    },
  );

  assert.equal(state.ended, false);
  assert.equal(state.participants.size, 0);
});

test("reconstructHuddleState keeps real joins active when START aged out", () => {
  const state = reconstructHuddleState(
    [lifecycleEvent(48101, { tags: [["p", PARTICIPANT]] })],
    HUDDLE_ID,
    { nowMs: NOW_SECONDS * 1000 },
  );

  assert.equal(state.ended, false);
  assert.equal(state.startCreatedAt, null);
  assert.deepEqual([...state.participants], [PARTICIPANT]);
});

test("reconstructHuddleState treats empty truncated history as inconclusive", () => {
  const events = [
    lifecycleEvent(48100, { created_at: NOW_SECONDS - 100 }),
    lifecycleEvent(48101, {
      created_at: NOW_SECONDS - 99,
      tags: [["p", CREATOR]],
    }),
  ];
  for (let index = 0; index < 50; index += 1) {
    const participant = `participant-${index}`;
    events.push(
      lifecycleEvent(48101, {
        created_at: NOW_SECONDS - 50 + index,
        tags: [["p", participant]],
      }),
      lifecycleEvent(48102, {
        created_at: NOW_SECONDS - 50 + index,
        tags: [["p", participant]],
      }),
    );
  }

  const state = reconstructHuddleState(events.slice(-100), HUDDLE_ID, {
    nowMs: NOW_SECONDS * 1000,
  });

  assert.equal(state.ended, false);
  assert.equal(state.startCreatedAt, null);
  assert.equal(state.participants.size, 0);
});

test("reconstructHuddleState keeps a skew-retained START inconclusive when history is truncated", () => {
  const events = [lifecycleEvent(48100)];
  for (let index = 0; index < 49; index += 1) {
    const participant = `participant-${index}`;
    events.push(
      lifecycleEvent(48101, {
        created_at: NOW_SECONDS - 100 + index * 2,
        tags: [["p", participant]],
      }),
      lifecycleEvent(48102, {
        created_at: NOW_SECONDS - 99 + index * 2,
        tags: [["p", participant]],
      }),
    );
  }
  events.push(
    lifecycleEvent(48102, {
      created_at: NOW_SECONDS - 1,
      tags: [["p", CREATOR]],
    }),
  );

  const state = reconstructHuddleState(events, HUDDLE_ID, {
    historyMayBeTruncated: true,
    nowMs: NOW_SECONDS * 1000,
  });

  assert.equal(events.length, 100);
  assert.equal(state.ended, false);
  assert.equal(state.participants.size, 0);
});

test("recordHuddleSubscriptionEvent preserves channel-wide truncation before huddle filtering", () => {
  const seenChannelEventIds = new Set();
  const seenHuddleEvents = new Map();
  const start = lifecycleEvent(48100);
  seenHuddleEvents.set(start.id, start);

  for (let index = 0; index < 99; index += 1) {
    const event = eventForHuddle(48101, `unrelated-huddle-${index}`, {
      id: `unrelated-event-${index}`,
      created_at: NOW_SECONDS - 100 + index,
      tags: [["p", `participant-${index}`]],
    });
    assert.equal(
      recordHuddleSubscriptionEvent(
        seenChannelEventIds,
        seenHuddleEvents,
        HUDDLE_ID,
        event,
      ),
      true,
    );
  }

  const retainedLeft = lifecycleEvent(48102, {
    id: "retained-left",
    created_at: NOW_SECONDS - 1,
    tags: [["p", CREATOR]],
  });
  recordHuddleSubscriptionEvent(
    seenChannelEventIds,
    seenHuddleEvents,
    HUDDLE_ID,
    retainedLeft,
  );

  const state = reconstructHuddleState(seenHuddleEvents.values(), HUDDLE_ID, {
    historyMayBeTruncated: seenChannelEventIds.size >= 100,
    nowMs: NOW_SECONDS * 1000,
  });

  assert.equal(seenChannelEventIds.size, 100);
  assert.equal(seenHuddleEvents.size, 2);
  assert.equal(state.ended, false);
  assert.equal(state.participants.size, 0);
});

test("createHuddleReplayTracker freezes truncation at the initial replay boundary", () => {
  const tracker = createHuddleReplayTracker();

  tracker.recordReplayEvent(lifecycleEvent(48100, { id: "initial-event" }));
  tracker.markReplayComplete();

  for (let index = 0; index < 99; index += 1) {
    tracker.recordReplayEvent(
      lifecycleEvent(48100, { id: `live-event-${index}` }),
    );
  }

  assert.equal(
    tracker.historyMayBeTruncated(),
    false,
    "live accumulation after initial replay must not mark history truncated",
  );

  const fullTracker = createHuddleReplayTracker();
  for (let index = 0; index < 100; index += 1) {
    fullTracker.recordReplayEvent(
      lifecycleEvent(48100, { id: `initial-event-${index}` }),
    );
  }

  assert.equal(
    fullTracker.historyMayBeTruncated(),
    true,
    "initial replay at the relay limit must still mark history truncated",
  );
});

test("createHuddleReplayTracker detects truncation during reconnect replay", () => {
  const tracker = createHuddleReplayTracker();
  const initialEvent = lifecycleEvent(48100, { id: "initial-event" });

  tracker.recordReplayEvent(initialEvent);
  tracker.markReplayComplete();
  assert.equal(tracker.replayComplete(), true);
  assert.equal(tracker.historyMayBeTruncated(), false);

  tracker.markReplayStarted([initialEvent]);
  assert.equal(tracker.replayComplete(), false);
  for (let index = 0; index < 100; index += 1) {
    tracker.recordReplayEvent(
      lifecycleEvent(48100, { id: `replay-event-${index}` }),
    );
  }
  tracker.markReplayComplete();

  assert.equal(tracker.replayComplete(), true);
  assert.equal(tracker.historyMayBeTruncated(), true);
});

test("createHuddleReplayTracker marks terminal replay failure as not in progress", () => {
  const tracker = createHuddleReplayTracker(2);
  tracker.markReplayStarted([lifecycleEvent(48100, { id: "retained-start" })]);
  tracker.recordReplayEvent(
    eventForHuddle(48100, "unrelated-huddle-1", { id: "unrelated-1" }),
  );
  tracker.recordReplayEvent(
    eventForHuddle(48100, "unrelated-huddle-2", { id: "unrelated-2" }),
  );

  tracker.markReplayFailed();

  assert.equal(tracker.replayComplete(), false);
  assert.equal(tracker.replayInProgress(), false);
  assert.equal(tracker.historyMayBeTruncated(), true);
});

test("createHuddleReplayTracker deduplicates deliveries within a replay window", () => {
  const tracker = createHuddleReplayTracker();
  const duplicateEvent = lifecycleEvent(48100, { id: "duplicate-event" });

  for (let index = 0; index < 100; index += 1) {
    tracker.recordReplayEvent(duplicateEvent);
  }

  assert.equal(tracker.historyMayBeTruncated(), false);

  for (let index = 0; index < 99; index += 1) {
    tracker.recordReplayEvent(
      lifecycleEvent(48100, { id: `unique-event-${index}` }),
    );
  }

  assert.equal(tracker.historyMayBeTruncated(), true);
});

test("createHuddleReplayTracker scopes truncation to replayed event ids", () => {
  const tracker = createHuddleReplayTracker(2);
  const replayedStart = lifecycleEvent(48100, { id: "replayed-start" });
  const replayedLeft = lifecycleEvent(48102, {
    id: "replayed-left",
    tags: [["p", CREATOR]],
  });

  tracker.recordReplayEvent(replayedStart);
  tracker.recordReplayEvent(replayedLeft);
  tracker.markReplayComplete();

  assert.equal(tracker.historyMayBeTruncated(), true);
  assert.equal(
    tracker.historyMayBeTruncatedForEvents([replayedStart, replayedLeft]),
    true,
  );

  const liveStart = lifecycleEvent(48100, {
    id: "live-start",
    created_at: NOW_SECONDS + 1,
  });
  const liveLeft = lifecycleEvent(48102, {
    id: "live-left",
    created_at: NOW_SECONDS + 2,
    tags: [["p", CREATOR]],
  });

  assert.equal(
    tracker.historyMayBeTruncatedForEvents([liveStart, liveLeft]),
    false,
  );
  assert.equal(
    reconstructHuddleState([liveStart, liveLeft], HUDDLE_ID, {
      historyMayBeTruncated: tracker.historyMayBeTruncatedForEvents([
        liveStart,
        liveLeft,
      ]),
      nowMs: (NOW_SECONDS + 2) * 1000,
    }).ended,
    false,
  );
});

test("createHuddleReplayTracker preserves replay gaps for pre-existing huddles", () => {
  const tracker = createHuddleReplayTracker(2);
  const retainedStart = lifecycleEvent(48100, {
    id: "retained-start",
    created_at: NOW_SECONDS - 10,
  });

  tracker.recordReplayEvent(retainedStart);
  tracker.markReplayComplete();

  tracker.markReplayStarted([retainedStart]);
  tracker.recordReplayEvent(
    eventForHuddle(48100, "unrelated-huddle-1", { id: "unrelated-1" }),
  );
  tracker.recordReplayEvent(
    eventForHuddle(48100, "unrelated-huddle-2", { id: "unrelated-2" }),
  );
  tracker.markReplayComplete();

  const retainedLeft = lifecycleEvent(48102, {
    id: "retained-left",
    created_at: NOW_SECONDS + 1,
    tags: [["p", CREATOR]],
  });

  assert.equal(
    tracker.historyMayBeTruncatedForEvents([retainedStart, retainedLeft]),
    true,
  );
  assert.equal(
    reconstructHuddleState([retainedStart, retainedLeft], HUDDLE_ID, {
      historyMayBeTruncated: tracker.historyMayBeTruncatedForEvents([
        retainedStart,
        retainedLeft,
      ]),
      nowMs: (NOW_SECONDS + 1) * 1000,
    }).ended,
    false,
  );
});

test("createHuddleReplayTracker seeds initial replay gaps with a retained START", () => {
  const tracker = createHuddleReplayTracker(2);
  const retainedStart = lifecycleEvent(48100, {
    id: "retained-start",
    created_at: NOW_SECONDS - 10,
  });

  tracker.markReplayStarted([retainedStart]);
  tracker.recordReplayEvent(
    eventForHuddle(48100, "unrelated-huddle-1", { id: "unrelated-1" }),
  );
  tracker.recordReplayEvent(
    eventForHuddle(48100, "unrelated-huddle-2", { id: "unrelated-2" }),
  );

  const retainedLeft = lifecycleEvent(48102, {
    id: "retained-left",
    created_at: NOW_SECONDS + 1,
    tags: [["p", CREATOR]],
  });

  assert.equal(
    tracker.historyMayBeTruncatedForEvents([retainedStart, retainedLeft]),
    true,
  );
  assert.equal(
    reconstructHuddleState([retainedStart, retainedLeft], HUDDLE_ID, {
      historyMayBeTruncated: tracker.historyMayBeTruncatedForEvents([
        retainedStart,
        retainedLeft,
      ]),
      nowMs: (NOW_SECONDS + 1) * 1000,
    }).ended,
    false,
  );
});

test("createHuddleReplayTracker preserves truncation when replay restarts before EOSE", () => {
  const tracker = createHuddleReplayTracker(2);
  const retainedStart = lifecycleEvent(48100, {
    id: "retained-start",
    created_at: NOW_SECONDS - 10,
  });

  tracker.markReplayStarted([retainedStart]);
  tracker.recordReplayEvent(
    eventForHuddle(48100, "unrelated-huddle-1", { id: "unrelated-1" }),
  );
  tracker.recordReplayEvent(
    eventForHuddle(48100, "unrelated-huddle-2", { id: "unrelated-2" }),
  );

  tracker.markReplayStarted([retainedStart]);

  const retainedLeft = lifecycleEvent(48102, {
    id: "retained-left",
    created_at: NOW_SECONDS + 1,
    tags: [["p", CREATOR]],
  });

  assert.equal(
    tracker.historyMayBeTruncatedForEvents([retainedStart, retainedLeft]),
    true,
  );
  assert.equal(
    reconstructHuddleState([retainedStart, retainedLeft], HUDDLE_ID, {
      historyMayBeTruncated: tracker.historyMayBeTruncatedForEvents([
        retainedStart,
        retainedLeft,
      ]),
      nowMs: (NOW_SECONDS + 1) * 1000,
    }).ended,
    false,
  );
});

test("createHuddleReplayTracker keeps empty rosters joinable during reconnect replay", () => {
  const tracker = createHuddleReplayTracker();
  const events = [
    lifecycleEvent(48100, { created_at: NOW_SECONDS - 60 * 60 - 1 }),
    lifecycleEvent(48102, {
      created_at: NOW_SECONDS - 60 * 60,
      tags: [["p", CREATOR]],
    }),
  ];

  tracker.markReplayComplete();
  assert.equal(
    reconstructHuddleState(events, HUDDLE_ID, {
      nowMs: NOW_SECONDS * 1000,
      replayComplete: tracker.replayComplete(),
    }).ended,
    true,
  );

  tracker.markReplayStarted();
  assert.equal(
    reconstructHuddleState(events, HUDDLE_ID, {
      nowMs: NOW_SECONDS * 1000,
      replayComplete: tracker.replayComplete(),
      replayInProgress: tracker.replayInProgress(),
    }).ended,
    false,
  );

  tracker.markReplayFailed();
  assert.equal(
    reconstructHuddleState(events, HUDDLE_ID, {
      nowMs: NOW_SECONDS * 1000,
      replayComplete: tracker.replayComplete(),
      replayInProgress: tracker.replayInProgress(),
    }).ended,
    true,
  );
});

test("selectActiveHuddleState does not resurrect an older incomplete huddle", () => {
  const olderHuddleId = "older-huddle";
  const newerHuddleId = "newer-huddle";

  const selected = selectActiveHuddleState(
    [
      eventForHuddle(48100, olderHuddleId, {
        created_at: NOW_SECONDS - 20,
      }),
      eventForHuddle(48101, olderHuddleId, {
        created_at: NOW_SECONDS - 19,
        tags: [["p", PARTICIPANT]],
      }),
      eventForHuddle(48100, newerHuddleId, {
        created_at: NOW_SECONDS - 10,
      }),
      eventForHuddle(48101, newerHuddleId, {
        created_at: NOW_SECONDS - 9,
        tags: [["p", PARTICIPANT]],
      }),
      eventForHuddle(48103, newerHuddleId, {
        created_at: NOW_SECONDS - 8,
      }),
    ],
    { nowMs: NOW_SECONDS * 1000 },
  );

  assert.equal(selected, null);
});

test("selectActiveHuddleState does not fall back to an older start-only room after a newer relay room ends", () => {
  const startOnlyHuddleId = "start-only-huddle";
  const endedHuddleId = "ended-huddle";
  const selected = selectActiveHuddleState(
    [
      eventForHuddle(48100, startOnlyHuddleId, {
        created_at: NOW_SECONDS - 20,
      }),
      eventForHuddle(48100, endedHuddleId, {
        created_at: NOW_SECONDS - 10,
      }),
      eventForHuddle(48101, endedHuddleId, {
        created_at: NOW_SECONDS - 9,
        tags: [["p", PARTICIPANT]],
      }),
      eventForHuddle(48103, endedHuddleId, {
        created_at: NOW_SECONDS - 8,
      }),
    ],
    { nowMs: NOW_SECONDS * 1000 },
  );

  assert.equal(selected, null);
});

test("selectActiveHuddleState does not fall back to an older start-only room after a newer start-only room ends", () => {
  const olderHuddleId = "older-huddle";
  const newerHuddleId = "newer-huddle";
  const selected = selectActiveHuddleState(
    [
      eventForHuddle(48100, olderHuddleId, {
        created_at: NOW_SECONDS - 20,
      }),
      eventForHuddle(48100, newerHuddleId, {
        created_at: NOW_SECONDS - 10,
      }),
      eventForHuddle(48103, newerHuddleId, {
        created_at: NOW_SECONDS - 9,
      }),
    ],
    { nowMs: NOW_SECONDS * 1000 },
  );

  assert.equal(selected, null);
});

test("selectActiveHuddleState does not reshow a locally suppressed relay huddle", () => {
  const startOnlyHuddleId = "start-only-huddle";
  const suppressedHuddleId = "suppressed-huddle";
  const selected = selectActiveHuddleState(
    [
      eventForHuddle(48100, startOnlyHuddleId, {
        created_at: NOW_SECONDS - 20,
      }),
      eventForHuddle(48100, suppressedHuddleId, {
        created_at: NOW_SECONDS - 10,
      }),
      eventForHuddle(48101, suppressedHuddleId, {
        created_at: NOW_SECONDS - 9,
        tags: [["p", PARTICIPANT]],
      }),
    ],
    {
      nowMs: NOW_SECONDS * 1000,
      suppressedEphemeralChannelId: suppressedHuddleId,
    },
  );

  assert.equal(selected, null);
});

test("selectActiveHuddleState lets the local current huddle override suppression", () => {
  const selected = selectActiveHuddleState(
    [
      eventForHuddle(48100, HUDDLE_ID, {
        created_at: NOW_SECONDS - 10,
      }),
      eventForHuddle(48101, HUDDLE_ID, {
        created_at: NOW_SECONDS - 9,
        tags: [["p", PARTICIPANT]],
      }),
    ],
    {
      activeEphemeralChannelId: HUDDLE_ID,
      nowMs: NOW_SECONDS * 1000,
      suppressedEphemeralChannelId: HUDDLE_ID,
    },
  );

  assert.equal(selected?.ephemeralChannelId, HUDDLE_ID);
  assert.equal(selected?.state.ended, false);
});

test("selectActiveHuddleState preserves a truncated huddle when the creator JOIN is missing", () => {
  const selected = selectActiveHuddleState(
    [
      eventForHuddle(48100, HUDDLE_ID, {
        created_at: NOW_SECONDS - 10,
        pubkey: CREATOR,
      }),
      eventForHuddle(48101, HUDDLE_ID, {
        created_at: NOW_SECONDS - 9,
        tags: [["p", PARTICIPANT]],
      }),
      eventForHuddle(48102, HUDDLE_ID, {
        created_at: NOW_SECONDS - 8,
        tags: [["p", PARTICIPANT]],
      }),
    ],
    { historyMayBeTruncated: true, nowMs: NOW_SECONDS * 1000 },
  );

  assert.ok(selected);
  assert.equal(selected?.ephemeralChannelId, HUDDLE_ID);
  assert.equal(selected?.state.ended, false);
  assert.deepEqual([...selected.state.participants], [CREATOR]);
});

test("selectActiveHuddleState keeps a newer ended-room barrier after an older room LEFT", () => {
  const olderHuddleId = "older-huddle";
  const newerHuddleId = "newer-huddle";
  const selected = selectActiveHuddleState(
    [
      eventForHuddle(48100, olderHuddleId, {
        created_at: NOW_SECONDS - 20,
      }),
      eventForHuddle(48101, olderHuddleId, {
        created_at: NOW_SECONDS - 19,
        tags: [["p", CREATOR]],
      }),
      eventForHuddle(48101, olderHuddleId, {
        created_at: NOW_SECONDS - 18,
        tags: [["p", PARTICIPANT]],
      }),
      eventForHuddle(48100, newerHuddleId, {
        created_at: NOW_SECONDS - 10,
      }),
      eventForHuddle(48101, newerHuddleId, {
        created_at: NOW_SECONDS - 9,
        tags: [["p", PARTICIPANT]],
      }),
      eventForHuddle(48103, newerHuddleId, {
        created_at: NOW_SECONDS - 8,
      }),
      eventForHuddle(48102, olderHuddleId, {
        created_at: NOW_SECONDS - 1,
        tags: [["p", PARTICIPANT]],
      }),
    ],
    { nowMs: NOW_SECONDS * 1000 },
  );

  assert.equal(selected, null);
});

test("selectActiveHuddleState keeps a newer ended-room barrier before an older live relay participant", () => {
  const olderHuddleId = "older-huddle";
  const newerHuddleId = "newer-huddle";
  const selected = selectActiveHuddleState(
    [
      eventForHuddle(48100, olderHuddleId, {
        created_at: NOW_SECONDS - 20,
      }),
      eventForHuddle(48101, olderHuddleId, {
        created_at: NOW_SECONDS - 19,
        tags: [["p", CREATOR]],
      }),
      eventForHuddle(48100, newerHuddleId, {
        created_at: NOW_SECONDS - 10,
      }),
      eventForHuddle(48103, newerHuddleId, {
        created_at: NOW_SECONDS - 9,
      }),
    ],
    { nowMs: NOW_SECONDS * 1000 },
  );

  assert.equal(selected, null);
});

test("selectActiveHuddleState orders relay lifecycle evidence across skewed START clocks", () => {
  const olderHuddleId = "older-huddle";
  const newerHuddleId = "newer-huddle";
  const selected = selectActiveHuddleState(
    [
      eventForHuddle(48100, olderHuddleId, {
        created_at: NOW_SECONDS + 10,
      }),
      eventForHuddle(48101, olderHuddleId, {
        created_at: NOW_SECONDS - 20,
        tags: [["p", PARTICIPANT]],
      }),
      eventForHuddle(48100, newerHuddleId, {
        created_at: NOW_SECONDS - 10,
      }),
      eventForHuddle(48101, newerHuddleId, {
        created_at: NOW_SECONDS - 9,
        tags: [["p", PARTICIPANT]],
      }),
      eventForHuddle(48103, newerHuddleId, {
        created_at: NOW_SECONDS - 3,
      }),
    ],
    { nowMs: NOW_SECONDS * 1000 },
  );

  assert.equal(selected, null);
});

test("selectActiveHuddleState ignores a future-skewed END when ordering rooms", () => {
  const endedHuddleId = "ended-huddle";
  const liveHuddleId = "live-huddle";
  const selected = selectActiveHuddleState(
    [
      eventForHuddle(48100, endedHuddleId, {
        created_at: NOW_SECONDS - 30,
      }),
      eventForHuddle(48101, endedHuddleId, {
        created_at: NOW_SECONDS - 20,
        tags: [["p", PARTICIPANT]],
      }),
      eventForHuddle(48103, endedHuddleId, {
        created_at: NOW_SECONDS + 15 * 60,
      }),
      eventForHuddle(48100, liveHuddleId, {
        created_at: NOW_SECONDS - 10,
      }),
      eventForHuddle(48101, liveHuddleId, {
        created_at: NOW_SECONDS - 5,
        tags: [["p", PARTICIPANT]],
      }),
    ],
    { nowMs: NOW_SECONDS * 1000 },
  );

  assert.equal(selected?.ephemeralChannelId, liveHuddleId);
  assert.equal(selected?.state.ended, false);
});

test("selectActiveHuddleState ignores a delayed LEFT from an ended room", () => {
  const endedHuddleId = "ended-huddle";
  const liveHuddleId = "live-huddle";
  const selected = selectActiveHuddleState(
    [
      eventForHuddle(48100, endedHuddleId, {
        created_at: NOW_SECONDS - 6,
      }),
      eventForHuddle(48101, endedHuddleId, {
        created_at: NOW_SECONDS - 5,
        tags: [["p", PARTICIPANT]],
      }),
      eventForHuddle(48103, endedHuddleId, {
        created_at: NOW_SECONDS - 4,
      }),
      eventForHuddle(48100, liveHuddleId, {
        created_at: NOW_SECONDS - 3,
      }),
      eventForHuddle(48101, liveHuddleId, {
        created_at: NOW_SECONDS - 2,
        tags: [["p", PARTICIPANT]],
      }),
      eventForHuddle(48102, endedHuddleId, {
        created_at: NOW_SECONDS - 1,
        tags: [["p", PARTICIPANT]],
      }),
    ],
    { nowMs: NOW_SECONDS * 1000 },
  );

  assert.equal(selected?.ephemeralChannelId, liveHuddleId);
  assert.equal(selected?.state.ended, false);
});

test("selectActiveHuddleState prefers live relay evidence over a future-skewed START-only session", () => {
  const startOnlyHuddleId = "start-only-huddle";
  const relayActiveHuddleId = "relay-active-huddle";
  const selected = selectActiveHuddleState(
    [
      eventForHuddle(48100, startOnlyHuddleId, {
        created_at: NOW_SECONDS + 15 * 60,
      }),
      eventForHuddle(48100, relayActiveHuddleId, {
        created_at: NOW_SECONDS - 10,
      }),
      eventForHuddle(48101, relayActiveHuddleId, {
        created_at: NOW_SECONDS - 5,
        tags: [["p", PARTICIPANT]],
      }),
    ],
    { nowMs: NOW_SECONDS * 1000 },
  );

  assert.equal(selected?.ephemeralChannelId, relayActiveHuddleId);
  assert.equal(selected?.state.ended, false);
});

test("selectActiveHuddleState prefers live relay evidence over a future-skewed ended session", () => {
  const endedHuddleId = "ended-huddle";
  const relayActiveHuddleId = "relay-active-huddle";
  const selected = selectActiveHuddleState(
    [
      eventForHuddle(48100, endedHuddleId, {
        created_at: NOW_SECONDS + 15 * 60,
      }),
      eventForHuddle(48103, endedHuddleId, {
        created_at: NOW_SECONDS + 15 * 60 + 1,
      }),
      eventForHuddle(48100, relayActiveHuddleId, {
        created_at: NOW_SECONDS - 10,
      }),
      eventForHuddle(48101, relayActiveHuddleId, {
        created_at: NOW_SECONDS - 5,
        tags: [["p", PARTICIPANT]],
      }),
    ],
    { nowMs: NOW_SECONDS * 1000 },
  );

  assert.equal(selected?.ephemeralChannelId, relayActiveHuddleId);
  assert.equal(selected?.state.ended, false);
});

test("selectActiveHuddleState prefers a present relay huddle when JOIN timestamps tie", () => {
  const endedHuddleId = "ended-huddle";
  const liveHuddleId = "live-huddle";
  const selected = selectActiveHuddleState(
    [
      eventForHuddle(48100, endedHuddleId, {
        created_at: NOW_SECONDS - 10,
      }),
      eventForHuddle(48101, endedHuddleId, {
        created_at: NOW_SECONDS - 5,
        tags: [["p", PARTICIPANT]],
      }),
      eventForHuddle(48103, endedHuddleId, {
        created_at: NOW_SECONDS - 4,
      }),
      eventForHuddle(48100, liveHuddleId, {
        created_at: NOW_SECONDS - 9,
      }),
      eventForHuddle(48101, liveHuddleId, {
        created_at: NOW_SECONDS - 5,
        tags: [["p", PARTICIPANT]],
      }),
    ],
    { nowMs: NOW_SECONDS * 1000 },
  );

  assert.equal(selected?.ephemeralChannelId, liveHuddleId);
  assert.equal(selected?.state.ended, false);
});

test("selectActiveHuddleState preserves live relay evidence when its START aged out", () => {
  const startOnlyHuddleId = "start-only-huddle";
  const relayActiveHuddleId = "relay-active-huddle";
  const selected = selectActiveHuddleState(
    [
      eventForHuddle(48100, startOnlyHuddleId, {
        created_at: NOW_SECONDS + 15 * 60,
      }),
      eventForHuddle(48101, relayActiveHuddleId, {
        created_at: NOW_SECONDS - 5,
        tags: [["p", PARTICIPANT]],
      }),
    ],
    { nowMs: NOW_SECONDS * 1000 },
  );

  assert.equal(selected?.ephemeralChannelId, relayActiveHuddleId);
  assert.equal(selected?.state.ended, false);
});

test("selectActiveHuddleState keeps a truncated LEFT-only relay huddle selectable", () => {
  const selected = selectActiveHuddleState(
    [
      eventForHuddle(48102, HUDDLE_ID, {
        created_at: NOW_SECONDS - 1,
        id: "retained-left",
        tags: [["p", CREATOR]],
      }),
    ],
    { historyMayBeTruncated: true, nowMs: NOW_SECONDS * 1000 },
  );

  assert.equal(selected?.ephemeralChannelId, HUDDLE_ID);
  assert.equal(selected?.state.ended, false);
  assert.equal(selected?.state.participants.size, 0);
});

test("selectActiveHuddleState preserves a newer truncated LEFT-only huddle over older JOIN evidence", () => {
  const olderHuddleId = "older-huddle";
  const newerHuddleId = "newer-huddle";
  const selected = selectActiveHuddleState(
    [
      eventForHuddle(48100, olderHuddleId, {
        created_at: NOW_SECONDS - 30,
        pubkey: CREATOR,
      }),
      eventForHuddle(48101, olderHuddleId, {
        created_at: NOW_SECONDS - 29,
        tags: [["p", PARTICIPANT]],
      }),
      eventForHuddle(48102, olderHuddleId, {
        created_at: NOW_SECONDS - 28,
        tags: [["p", PARTICIPANT]],
      }),
      eventForHuddle(48102, olderHuddleId, {
        created_at: NOW_SECONDS - 27,
        tags: [["p", CREATOR]],
      }),
      eventForHuddle(48100, newerHuddleId, {
        created_at: NOW_SECONDS - 10,
        pubkey: CREATOR,
      }),
      eventForHuddle(48102, newerHuddleId, {
        created_at: NOW_SECONDS - 9,
        tags: [["p", CREATOR]],
      }),
    ],
    { historyMayBeTruncated: true, nowMs: NOW_SECONDS * 1000 },
  );

  assert.equal(selected?.ephemeralChannelId, newerHuddleId);
  assert.equal(selected?.state.ended, false);
  assert.equal(selected?.state.participants.size, 0);
});

test("selectActiveHuddleState keeps an empty relay huddle selectable", () => {
  const selected = selectActiveHuddleState(
    [
      eventForHuddle(48100, HUDDLE_ID, {
        created_at: NOW_SECONDS - 10,
      }),
      eventForHuddle(48102, HUDDLE_ID, {
        created_at: NOW_SECONDS - 9,
        tags: [["p", CREATOR]],
      }),
    ],
    { nowMs: NOW_SECONDS * 1000 },
  );

  assert.equal(selected?.ephemeralChannelId, HUDDLE_ID);
  assert.equal(selected?.state.ended, false);
  assert.equal(selected?.state.participants.size, 0);
});

test("selectActiveHuddleState keeps an empty relay huddle selectable until replay completes", () => {
  const selected = selectActiveHuddleState(
    [
      eventForHuddle(48100, HUDDLE_ID, {
        created_at: NOW_SECONDS - 10,
      }),
      eventForHuddle(48101, HUDDLE_ID, {
        created_at: NOW_SECONDS - 9,
        tags: [["p", PARTICIPANT]],
      }),
      eventForHuddle(48102, HUDDLE_ID, {
        created_at: NOW_SECONDS - 8,
        tags: [["p", PARTICIPANT]],
      }),
      eventForHuddle(48102, HUDDLE_ID, {
        created_at: NOW_SECONDS - 7,
        tags: [["p", CREATOR]],
      }),
    ],
    { nowMs: NOW_SECONDS * 1000, replayComplete: false },
  );

  assert.equal(selected?.ephemeralChannelId, HUDDLE_ID);
  assert.equal(selected?.state.ended, false);
  assert.equal(selected?.state.participants.size, 0);
});

test("selectActiveHuddleState defers START-only stale expiry during replay", () => {
  const selected = selectActiveHuddleState(
    [
      eventForHuddle(48100, HUDDLE_ID, {
        created_at: NOW_SECONDS - 60 * 60 - 1,
      }),
    ],
    {
      nowMs: NOW_SECONDS * 1000,
      replayComplete: false,
      replayInProgress: true,
    },
  );

  assert.equal(selected?.ephemeralChannelId, HUDDLE_ID);
  assert.equal(selected?.state.ended, false);
  assert.equal(selected?.state.staleDeadlineMs, null);
});

test("selectActiveHuddleState prefers a fresh START-only session over older empty relay history", () => {
  const endedHuddleId = "ended-huddle";
  const emptyRosterHuddleId = "empty-roster-huddle";
  const startOnlyHuddleId = "start-only-huddle";
  const selected = selectActiveHuddleState(
    [
      eventForHuddle(48100, endedHuddleId, {
        created_at: NOW_SECONDS - 20,
      }),
      eventForHuddle(48101, endedHuddleId, {
        created_at: NOW_SECONDS - 19,
        tags: [["p", PARTICIPANT]],
      }),
      eventForHuddle(48103, endedHuddleId, {
        created_at: NOW_SECONDS - 18,
      }),
      eventForHuddle(48100, emptyRosterHuddleId, {
        created_at: NOW_SECONDS - 10,
      }),
      eventForHuddle(48102, emptyRosterHuddleId, {
        created_at: NOW_SECONDS - 9,
        tags: [["p", CREATOR]],
      }),
      eventForHuddle(48100, startOnlyHuddleId, {
        created_at: NOW_SECONDS - 1,
      }),
    ],
    { nowMs: NOW_SECONDS * 1000 },
  );

  assert.equal(selected?.ephemeralChannelId, startOnlyHuddleId);
  assert.equal(selected?.state.ended, false);
});

test("selectActiveHuddleState keeps a suppressed START-only huddle as a newer barrier", () => {
  const olderHuddleId = "older-huddle";
  const suppressedHuddleId = "suppressed-huddle";
  const selected = selectActiveHuddleState(
    [
      eventForHuddle(48100, olderHuddleId, {
        created_at: NOW_SECONDS - 20,
      }),
      eventForHuddle(48100, suppressedHuddleId, {
        created_at: NOW_SECONDS - 10,
      }),
    ],
    {
      nowMs: NOW_SECONDS * 1000,
      suppressedEphemeralChannelId: suppressedHuddleId,
    },
  );

  assert.equal(selected, null);
});

test("selectActiveHuddleState does not compare START-only clocks to relay JOIN clocks", () => {
  const endedHuddleId = "ended-huddle";
  const startOnlyHuddleId = "start-only-huddle";
  const selected = selectActiveHuddleState(
    [
      eventForHuddle(48101, endedHuddleId, {
        created_at: NOW_SECONDS,
        tags: [["p", PARTICIPANT]],
      }),
      eventForHuddle(48103, endedHuddleId, {
        created_at: NOW_SECONDS + 1,
      }),
      eventForHuddle(48100, startOnlyHuddleId, {
        created_at: NOW_SECONDS - 60,
      }),
    ],
    { nowMs: NOW_SECONDS * 1000 },
  );

  assert.equal(selected?.ephemeralChannelId, startOnlyHuddleId);
  assert.equal(selected?.state.ended, false);
});

test("selectActiveHuddleState does not tier a departed JOIN participant as present", () => {
  const relayHistoryHuddleId = "relay-history-huddle";
  const startOnlyHuddleId = "start-only-huddle";
  const selected = selectActiveHuddleState(
    [
      eventForHuddle(48100, relayHistoryHuddleId, {
        created_at: NOW_SECONDS - 20,
      }),
      eventForHuddle(48101, relayHistoryHuddleId, {
        created_at: NOW_SECONDS - 19,
        tags: [["p", PARTICIPANT]],
      }),
      eventForHuddle(48102, relayHistoryHuddleId, {
        created_at: NOW_SECONDS - 18,
        tags: [["p", PARTICIPANT]],
      }),
      eventForHuddle(48100, startOnlyHuddleId, {
        created_at: NOW_SECONDS - 1,
      }),
    ],
    { nowMs: NOW_SECONDS * 1000 },
  );

  assert.equal(selected?.ephemeralChannelId, startOnlyHuddleId);
  assert.equal(selected?.state.ended, false);
});

test("selectActiveHuddleState preserves an inconclusive truncated relay huddle", () => {
  const selected = selectActiveHuddleState(
    [
      eventForHuddle(48100, HUDDLE_ID, {
        created_at: NOW_SECONDS - 10,
      }),
      eventForHuddle(48102, HUDDLE_ID, {
        created_at: NOW_SECONDS - 9,
        id: "creator-left",
        tags: [["p", CREATOR]],
      }),
      eventForHuddle(48101, HUDDLE_ID, {
        created_at: NOW_SECONDS - 8,
        id: "balanced-join",
        tags: [["p", PARTICIPANT]],
      }),
      eventForHuddle(48102, HUDDLE_ID, {
        created_at: NOW_SECONDS - 7,
        id: "balanced-left",
        tags: [["p", PARTICIPANT]],
      }),
    ],
    { historyMayBeTruncated: true, nowMs: NOW_SECONDS * 1000 },
  );

  assert.equal(selected?.ephemeralChannelId, HUDDLE_ID);
  assert.equal(selected?.state.ended, false);
  assert.equal(selected?.state.participants.size, 0);
});

test("reconstructHuddleState does not resurrect after an end event", () => {
  const state = reconstructHuddleState(
    [
      lifecycleEvent(48100),
      lifecycleEvent(48103, { created_at: NOW_SECONDS + 1 }),
      lifecycleEvent(48101, {
        created_at: NOW_SECONDS + 2,
        tags: [["p", PARTICIPANT]],
      }),
    ],
    HUDDLE_ID,
    { nowMs: (NOW_SECONDS + 2) * 1000 },
  );

  assert.equal(state.ended, true);
  assert.deepEqual([...state.participants], [CREATOR, PARTICIPANT]);
});

test("huddleStalenessDelayMs schedules just past the stale boundary", () => {
  assert.equal(
    huddleStalenessDelayMs((NOW_SECONDS + 10) * 1000 + 1, NOW_SECONDS * 1000),
    10_001,
  );
  assert.equal(huddleStalenessDelayMs(null, NOW_SECONDS * 1000), null);
});

test("huddleStalenessDelayMs caps oversized timer delays", () => {
  assert.equal(
    huddleStalenessDelayMs(
      NOW_SECONDS * 1000 + 3_000_000_000,
      NOW_SECONDS * 1000,
    ),
    2_147_483_647,
  );
});

test("huddleParticipantDisplayCount floors the locally current huddle", () => {
  assert.equal(huddleParticipantDisplayCount(new Set(), {}), 0);
  assert.equal(
    huddleParticipantDisplayCount(new Set(), { isCurrentHuddle: true }),
    1,
  );
  assert.equal(
    huddleParticipantDisplayCount(new Set([CREATOR, PARTICIPANT]), {
      isCurrentHuddle: true,
    }),
    2,
  );
});

test("resolveIdleHuddleTransition does not clear an unrelated displayed huddle", () => {
  const transition = resolveIdleHuddleTransition({
    activeEphemeralChannelId: null,
    displayedEphemeralChannelId: "visible-huddle",
    eventEphemeralChannelId: null,
    lastActiveEphemeralChannelId: "departing-huddle",
  });

  assert.equal(transition.shouldClearDisplayedHuddle, false);
  assert.equal(transition.suppressedEphemeralChannelId, "departing-huddle");
});

test("resolveIdleHuddleTransition clears the displayed huddle that went idle", () => {
  const transition = resolveIdleHuddleTransition({
    activeEphemeralChannelId: null,
    displayedEphemeralChannelId: "departing-huddle",
    eventEphemeralChannelId: null,
    lastActiveEphemeralChannelId: "departing-huddle",
  });

  assert.equal(transition.shouldClearDisplayedHuddle, true);
  assert.equal(transition.suppressedEphemeralChannelId, "departing-huddle");
});

test("huddleEventClearsSuppression ignores inconclusive participant LEFT", () => {
  assert.equal(huddleEventClearsSuppression(lifecycleEvent(48102)), false);
  assert.equal(huddleEventClearsSuppression(lifecycleEvent(48101)), true);
  assert.equal(huddleEventClearsSuppression(lifecycleEvent(48103)), true);
});

test("huddleEventClearsSuppressionForState clears participant LEFT only for active remaining rosters", () => {
  const leftEvent = lifecycleEvent(48102);

  assert.equal(
    huddleEventClearsSuppressionForState(leftEvent, {
      ended: false,
      participants: new Set([PARTICIPANT]),
      staleDeadlineMs: null,
      startCreatedAt: NOW_SECONDS,
    }),
    true,
  );
  assert.equal(
    huddleEventClearsSuppressionForState(leftEvent, {
      ended: false,
      participants: new Set(),
      staleDeadlineMs: null,
      startCreatedAt: NOW_SECONDS,
    }),
    false,
  );
  assert.equal(
    huddleEventClearsSuppressionForState(leftEvent, {
      ended: true,
      participants: new Set([PARTICIPANT]),
      staleDeadlineMs: null,
      startCreatedAt: NOW_SECONDS,
    }),
    false,
  );
});
