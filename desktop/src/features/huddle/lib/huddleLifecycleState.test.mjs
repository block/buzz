import assert from "node:assert/strict";
import test from "node:test";

import {
  huddleStalenessDelayMs,
  recordHuddleSubscriptionEvent,
  reconstructHuddleState,
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

test("reconstructHuddleState ends a fully drained huddle", () => {
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

  assert.equal(state.ended, true);
  assert.equal(state.participants.size, 0);
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

test("reconstructHuddleState keeps an old START active after a recent JOIN", () => {
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
  const startCreatedAt = NOW_SECONDS - 60 * 60 - 1;
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

  assert.equal(state.ended, true);
  assert.equal(state.participants.size, 0);
});

test("reconstructHuddleState drains participants under START clock skew", () => {
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

  assert.equal(state.ended, true);
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

test("selectActiveHuddleState prefers a fresh START-only session over terminal relay history", () => {
  const endedHuddleId = "ended-huddle";
  const drainedHuddleId = "drained-huddle";
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
      eventForHuddle(48100, drainedHuddleId, {
        created_at: NOW_SECONDS - 10,
      }),
      eventForHuddle(48102, drainedHuddleId, {
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
