/**
 * Tests for activityThreads — turn → thread-root resolution and grouping.
 *
 * A channel turn resolves through the LAST of its triggeringEventIds: a
 * trigger with no thread tags IS the root; a reply resolves through its root
 * marker. Heartbeat turns, triggerless turns, and failed fetches land in the
 * unthreaded bucket. Groups order most-recently-active first and carry a
 * per-agent turn-id allowlist for scene scoping.
 */

import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  buildThreadGroups,
  filterFramesByTurnIds,
  resolveTurnThread,
} from "./activityThreads.ts";

const T0 = Date.parse("2026-08-18T12:00:00.000Z");

function turn(id, overrides = {}) {
  return {
    id,
    startT: T0,
    triggeringEventIds: [],
    ...overrides,
  };
}

/** Minimal RelayEvent for the injected fetcher. */
function relayEvent(id, { content = "hello", tags = [], createdAt } = {}) {
  return {
    id,
    pubkey: "f".repeat(64),
    created_at: createdAt ?? Math.floor(T0 / 1000),
    kind: 9,
    tags,
    content,
    sig: "sig",
  };
}

function fetcherFor(events) {
  const byId = new Map(events.map((event) => [event.id, event]));
  return async (eventId) => byId.get(eventId) ?? null;
}

const ROOT_ID = "a".repeat(64);
const REPLY_ID = "b".repeat(64);

describe("resolveTurnThread", () => {
  it("treats a tagless trigger as the thread root itself", async () => {
    const fetchEvent = fetcherFor([
      relayEvent(ROOT_ID, { content: "Fix the login flow" }),
    ]);
    const resolution = await resolveTurnThread(
      turn("t1", { source: "channel", triggeringEventIds: [ROOT_ID] }),
      fetchEvent,
    );
    assert.equal(resolution.rootId, ROOT_ID);
  });

  it("resolves a direct reply (single reply tag) to its root", async () => {
    const fetchEvent = fetcherFor([
      relayEvent(REPLY_ID, {
        content: "a reply",
        // Direct reply to root: single "reply" marker, no "root" marker.
        tags: [["e", ROOT_ID, "", "reply"]],
      }),
    ]);
    const resolution = await resolveTurnThread(
      turn("t1", { source: "channel", triggeringEventIds: [REPLY_ID] }),
      fetchEvent,
    );
    assert.equal(resolution.rootId, ROOT_ID);
  });

  it("uses the LAST triggering event when a batch drained several", async () => {
    const otherId = "c".repeat(64);
    const fetchEvent = fetcherFor([
      relayEvent(otherId, { content: "first drained" }),
      relayEvent(ROOT_ID, { content: "last drained" }),
    ]);
    const resolution = await resolveTurnThread(
      turn("t1", {
        source: "channel",
        triggeringEventIds: [otherId, ROOT_ID],
      }),
      fetchEvent,
    );
    assert.equal(resolution.rootId, ROOT_ID);
  });

  it("resolves a nested reply via its root marker without fetching the root", async () => {
    const parentId = "d".repeat(64);
    let fetches = 0;
    const fetchEvent = async (eventId) => {
      fetches += 1;
      return eventId === REPLY_ID
        ? relayEvent(REPLY_ID, {
            content: "a nested reply",
            // Nested reply: root marker + reply marker to its parent.
            tags: [
              ["e", ROOT_ID, "", "root"],
              ["e", parentId, "", "reply"],
            ],
          })
        : null;
    };
    const resolution = await resolveTurnThread(
      turn("t1", { source: "channel", triggeringEventIds: [REPLY_ID] }),
      fetchEvent,
    );
    assert.equal(resolution.rootId, ROOT_ID);
    // Root id comes from the trigger's own tags — one fetch, no root fetch.
    assert.equal(fetches, 1);
  });

  it("returns unthreaded for heartbeat turns without fetching", async () => {
    let fetches = 0;
    const resolution = await resolveTurnThread(
      turn("t1", {
        source: "heartbeat",
        triggeringEventIds: [ROOT_ID],
      }),
      async () => {
        fetches += 1;
        return null;
      },
    );
    assert.equal(resolution.rootId, null);
    assert.equal(fetches, 0);
  });

  it("returns unthreaded for triggerless turns and failed fetches", async () => {
    const noTrigger = await resolveTurnThread(
      turn("t1", { source: "channel" }),
      async () => relayEvent(ROOT_ID),
    );
    assert.equal(noTrigger.rootId, null);

    const fetchMiss = await resolveTurnThread(
      turn("t2", { source: "channel", triggeringEventIds: [ROOT_ID] }),
      async () => null,
    );
    assert.equal(fetchMiss.rootId, null);
  });
});

describe("buildThreadGroups", () => {
  const resolutionA = { rootId: ROOT_ID };
  const resolutionB = { rootId: REPLY_ID };
  const unthreadedResolution = { rootId: null };

  it("groups turns across agents and orders threads by recency", () => {
    const turnsByAgent = new Map([
      [
        "agent1",
        [
          turn("t1", { startT: T0, endT: T0 + 1000 }),
          turn("t2", { startT: T0 + 5000, endT: T0 + 6000 }),
        ],
      ],
      ["agent2", [turn("t3", { startT: T0 + 2000, endT: T0 + 3000 })]],
    ]);
    const resolutions = new Map([
      ["t1", resolutionA],
      ["t2", resolutionB],
      ["t3", resolutionA],
    ]);

    const groups = buildThreadGroups(turnsByAgent, resolutions);

    assert.equal(groups.threads.length, 2);
    // Thread B's turn ended latest → first.
    assert.equal(groups.threads[0].rootId, REPLY_ID);
    assert.equal(groups.threads[1].rootId, ROOT_ID);
    assert.equal(groups.threads[1].turnCount, 2);
    assert.deepEqual(
      [...groups.threads[1].turnIdsByAgent.get("agent1")],
      ["t1"],
    );
    assert.deepEqual(
      [...groups.threads[1].turnIdsByAgent.get("agent2")],
      ["t3"],
    );
    assert.equal(groups.unthreaded, null);
  });

  it("collects unthreaded turns separately and skips unresolved turns", () => {
    const turnsByAgent = new Map([
      [
        "agent1",
        [
          turn("t1", { startT: T0, endT: T0 + 1000 }),
          turn("pending", { startT: T0 + 2000 }),
        ],
      ],
    ]);
    const resolutions = new Map([["t1", unthreadedResolution]]);

    const groups = buildThreadGroups(turnsByAgent, resolutions);

    assert.equal(groups.threads.length, 0);
    assert.equal(groups.unthreaded?.turnCount, 1);
    // "pending" has no resolution yet → in neither bucket.
    assert.deepEqual(
      [...groups.unthreaded.turnIdsByAgent.get("agent1")],
      ["t1"],
    );
  });

  it("tracks agents with open turns per group", () => {
    const turnsByAgent = new Map([
      [
        "agent1",
        [
          turn("done", { startT: T0, endT: T0 + 1000 }),
          turn("open", { startT: T0 + 2000 }), // no endT: still running
        ],
      ],
      ["agent2", [turn("also-done", { startT: T0, endT: T0 + 500 })]],
    ]);
    const resolutions = new Map([
      ["done", resolutionA],
      ["open", resolutionA],
      ["also-done", resolutionA],
    ]);

    const groups = buildThreadGroups(turnsByAgent, resolutions);
    assert.deepEqual([...groups.threads[0].agentsWithOpenTurn], ["agent1"]);
  });

  it("uses an open turn's startT for recency", () => {
    const turnsByAgent = new Map([
      [
        "agent1",
        [
          turn("closed", { startT: T0, endT: T0 + 1000 }),
          turn("live", { startT: T0 + 9000 }), // endT undefined: live
        ],
      ],
    ]);
    const resolutions = new Map([
      ["closed", resolutionA],
      ["live", resolutionB],
    ]);

    const groups = buildThreadGroups(turnsByAgent, resolutions);
    assert.equal(groups.threads[0].rootId, REPLY_ID);
    assert.equal(groups.threads[0].lastT, T0 + 9000);
  });
});

describe("filterFramesByTurnIds", () => {
  it("keeps only frames whose turnId is allowed; drops turnless frames", () => {
    const frames = [
      { seq: 1, turnId: "t1", kind: "turn_started" },
      { seq: 2, turnId: null, kind: "connection" },
      { seq: 3, turnId: "t2", kind: "acp_read" },
      { seq: 4, turnId: "t1", kind: "turn_completed" },
    ];
    const filtered = filterFramesByTurnIds(frames, new Set(["t1"]));
    assert.deepEqual(
      filtered.map((frame) => frame.seq),
      [1, 4],
    );
  });
});
