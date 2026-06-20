import assert from "node:assert/strict";
import test from "node:test";

import {
  BOTTOM_THRESHOLD_PX,
  buildDayGroupBoundaries,
  isDeferredTimelineSnapshotStale,
  isNearBottomMetrics,
  isRenderedTimelineBehindHistoryPrepend,
  resolveDeepLinkTarget,
  selectDeferredListRenderState,
  selectLatestMessageAutoScrollBehavior,
  selectLatestMessageKey,
  selectTimelineBodySurface,
  selectTimelineIntroSurface,
} from "./timelineSnapshot.ts";

// Local-midnight unix-second timestamps so isSameDay (local time) is stable
// regardless of the machine's timezone.
function dayAt(year, month, day, hour = 12) {
  return Math.floor(
    new Date(year, month - 1, day, hour, 0, 0).getTime() / 1_000,
  );
}

function message(overrides) {
  return {
    id: "message",
    renderKey: undefined,
    createdAt: dayAt(2026, 6, 14),
    pubkey: "author",
    author: "Author",
    avatarUrl: null,
    role: undefined,
    personaDisplayName: undefined,
    time: "12:00 PM",
    body: "body",
    parentId: null,
    rootId: null,
    depth: 0,
    accent: false,
    pending: undefined,
    edited: false,
    kind: 9,
    tags: [],
    reactions: undefined,
    ...overrides,
  };
}

// --- sticky-bottom autoscroll -------------------------------------------------

test("isNearBottomMetrics: true when within threshold of the bottom", () => {
  // distance = scrollHeight - clientHeight - scrollTop = 1000 - 600 - 380 = 20
  assert.equal(
    isNearBottomMetrics({
      scrollHeight: 1_000,
      clientHeight: 600,
      scrollTop: 380,
    }),
    true,
  );
});

test("isNearBottomMetrics: true exactly at the threshold boundary", () => {
  const scrollTop = 1_000 - 600 - BOTTOM_THRESHOLD_PX; // distance === threshold
  assert.equal(
    isNearBottomMetrics({ scrollHeight: 1_000, clientHeight: 600, scrollTop }),
    true,
  );
});

test("isNearBottomMetrics: false when scrolled up beyond the threshold", () => {
  // distance = 1000 - 600 - 100 = 300 > 72
  assert.equal(
    isNearBottomMetrics({
      scrollHeight: 1_000,
      clientHeight: 600,
      scrollTop: 100,
    }),
    false,
  );
});

test("selectLatestMessageKey: prefers renderKey, falls back to id, undefined when empty", () => {
  assert.equal(selectLatestMessageKey([]), undefined);
  assert.equal(
    selectLatestMessageKey([message({ id: "a" }), message({ id: "b" })]),
    "b",
  );
  assert.equal(
    selectLatestMessageKey([message({ id: "b", renderKey: "local-b" })]),
    "local-b",
  );
});

test("selectLatestMessageKey: detects a newly arrived latest message", () => {
  const before = [message({ id: "a" }), message({ id: "b" })];
  const after = [
    ...before,
    message({ id: "c", createdAt: dayAt(2026, 6, 14, 13) }),
  ];
  assert.notEqual(
    selectLatestMessageKey(before),
    selectLatestMessageKey(after),
  );
});

test("selectLatestMessageAutoScrollBehavior: keeps sticky timelines pinned automatically", () => {
  assert.equal(
    selectLatestMessageAutoScrollBehavior({
      hasExplicitBottomRequest: false,
      isAtBottom: false,
      shouldStickToBottom: true,
      targetMessageId: null,
    }),
    "auto",
  );
  assert.equal(
    selectLatestMessageAutoScrollBehavior({
      hasExplicitBottomRequest: false,
      isAtBottom: true,
      shouldStickToBottom: false,
      targetMessageId: null,
    }),
    "auto",
  );
});

test("selectLatestMessageAutoScrollBehavior: explicit send requests smooth bottom scroll", () => {
  assert.equal(
    selectLatestMessageAutoScrollBehavior({
      hasExplicitBottomRequest: true,
      isAtBottom: false,
      shouldStickToBottom: false,
      targetMessageId: null,
    }),
    "smooth",
  );
});

test("selectLatestMessageAutoScrollBehavior: self-authored inserts do not imply scroll", () => {
  assert.equal(
    selectLatestMessageAutoScrollBehavior({
      hasExplicitBottomRequest: false,
      isAtBottom: false,
      shouldStickToBottom: false,
      targetMessageId: null,
    }),
    null,
  );
});

test("selectLatestMessageAutoScrollBehavior: target navigation suppresses latest-message autoscroll", () => {
  assert.equal(
    selectLatestMessageAutoScrollBehavior({
      hasExplicitBottomRequest: true,
      isAtBottom: true,
      shouldStickToBottom: true,
      targetMessageId: "message-a",
    }),
    null,
  );
});

// --- day dividers -------------------------------------------------------------

test("buildDayGroupBoundaries: empty snapshot has no groups", () => {
  assert.deepEqual(buildDayGroupBoundaries([]), []);
});

test("buildDayGroupBoundaries: messages on one day form a single group", () => {
  const messages = [
    message({ id: "a", createdAt: dayAt(2026, 6, 14, 9) }),
    message({ id: "b", createdAt: dayAt(2026, 6, 14, 10) }),
    message({ id: "c", createdAt: dayAt(2026, 6, 14, 23) }),
  ];
  const groups = buildDayGroupBoundaries(messages);
  assert.equal(groups.length, 1);
  assert.deepEqual(
    { startIndex: groups[0].startIndex, count: groups[0].count },
    { startIndex: 0, count: 3 },
  );
});

test("buildDayGroupBoundaries: a day boundary opens a new group", () => {
  const messages = [
    message({ id: "a", createdAt: dayAt(2026, 6, 13, 22) }),
    message({ id: "b", createdAt: dayAt(2026, 6, 14, 1) }),
    message({ id: "c", createdAt: dayAt(2026, 6, 14, 2) }),
    message({ id: "d", createdAt: dayAt(2026, 6, 15, 8) }),
  ];
  const groups = buildDayGroupBoundaries(messages);
  assert.deepEqual(
    groups.map((g) => ({ startIndex: g.startIndex, count: g.count })),
    [
      { startIndex: 0, count: 1 },
      { startIndex: 1, count: 2 },
      { startIndex: 3, count: 1 },
    ],
  );
});

test("buildDayGroupBoundaries: group counts always sum to message count", () => {
  const messages = [
    message({ id: "a", createdAt: dayAt(2026, 6, 13) }),
    message({ id: "b", createdAt: dayAt(2026, 6, 14) }),
    message({ id: "c", createdAt: dayAt(2026, 6, 14) }),
  ];
  const total = buildDayGroupBoundaries(messages).reduce(
    (n, g) => n + g.count,
    0,
  );
  assert.equal(total, messages.length);
});

test("buildDayGroupBoundaries: same-day group key is stable across a prepend", () => {
  // The day section is keyed by this value; if it changes when an older
  // message lands on the same calendar day, React remounts the whole section
  // on every scroll-up prepend — the timeline flicker. The key must depend on
  // the calendar day, not the first message's exact timestamp.
  const before = buildDayGroupBoundaries([
    message({ id: "b", createdAt: dayAt(2026, 6, 14, 9) }),
    message({ id: "c", createdAt: dayAt(2026, 6, 14, 10) }),
  ]);
  const afterPrepend = buildDayGroupBoundaries([
    message({ id: "a", createdAt: dayAt(2026, 6, 14, 8) }),
    message({ id: "b", createdAt: dayAt(2026, 6, 14, 9) }),
    message({ id: "c", createdAt: dayAt(2026, 6, 14, 10) }),
  ]);
  assert.equal(before[0].key, afterPrepend[0].key);
});

test("buildDayGroupBoundaries: distinct calendar days get distinct keys", () => {
  const groups = buildDayGroupBoundaries([
    message({ id: "a", createdAt: dayAt(2026, 6, 13, 12) }),
    message({ id: "b", createdAt: dayAt(2026, 6, 14, 12) }),
  ]);
  assert.notEqual(groups[0].key, groups[1].key);
});

// --- jump-to-message deep links ----------------------------------------------

test("resolveDeepLinkTarget: unresolved with no target", () => {
  const messages = [message({ id: "a" })];
  assert.deepEqual(resolveDeepLinkTarget(messages, null), {
    resolved: false,
    index: -1,
  });
  assert.deepEqual(resolveDeepLinkTarget(messages, undefined), {
    resolved: false,
    index: -1,
  });
});

test("resolveDeepLinkTarget: resolves a present target to its index", () => {
  const messages = [
    message({ id: "a" }),
    message({ id: "b" }),
    message({ id: "c" }),
  ];
  assert.deepEqual(resolveDeepLinkTarget(messages, "b"), {
    resolved: true,
    index: 1,
  });
});

test("resolveDeepLinkTarget: unresolved when the target is not in the snapshot", () => {
  const messages = [message({ id: "a" }), message({ id: "b" })];
  assert.deepEqual(resolveDeepLinkTarget(messages, "missing"), {
    resolved: false,
    index: -1,
  });
});

// --- shared-snapshot / no-tearing guarantee ----------------------------------
//
// All three must-keep decisions must read off the SAME snapshot. If the deep-link
// decision reads a fresh snapshot while the rendered list / scroll math still
// read a stale one, the jump fires against a row that hasn't committed and
// silently fails.

test("no-tearing: a target only in the fresh snapshot does NOT resolve against the stale one", () => {
  const stale = [message({ id: "a" }), message({ id: "b" })];
  const fresh = [
    ...stale,
    message({ id: "target", createdAt: dayAt(2026, 6, 15) }),
  ];

  // Reading the deep link against the stale snapshot (what the painted DOM
  // still shows) must report unresolved — you can't scroll to an uncommitted row.
  assert.equal(resolveDeepLinkTarget(stale, "target").resolved, false);
  // Against the fresh snapshot it resolves — proving the gate is which snapshot
  // you feed, not the function.
  assert.equal(resolveDeepLinkTarget(fresh, "target").resolved, true);
});

test("no-tearing: all three decisions agree when fed one shared snapshot", () => {
  const snapshot = [
    message({ id: "a", createdAt: dayAt(2026, 6, 13) }),
    message({ id: "b", createdAt: dayAt(2026, 6, 14) }),
    message({
      id: "target",
      renderKey: "rk-target",
      createdAt: dayAt(2026, 6, 14, 18),
    }),
  ];

  // Deep link resolves to the last index...
  const link = resolveDeepLinkTarget(snapshot, "target");
  // ...the day grouping covers that same index...
  const groups = buildDayGroupBoundaries(snapshot);
  const coveredCount = groups.reduce((n, g) => n + g.count, 0);
  // ...and the sticky-bottom latest-key points at that same final message.
  const latestKey = selectLatestMessageKey(snapshot);

  assert.equal(link.index, snapshot.length - 1);
  assert.equal(coveredCount, snapshot.length);
  assert.equal(latestKey, snapshot[snapshot.length - 1].renderKey);
});

test("no-tearing: stale snapshot keeps all three decisions internally consistent", () => {
  // Feeding the stale list everywhere keeps the decisions consistent with each
  // other — none of them see the uncommitted row.
  const stale = [
    message({ id: "a", createdAt: dayAt(2026, 6, 14, 9) }),
    message({ id: "b", createdAt: dayAt(2026, 6, 14, 10) }),
  ];

  const link = resolveDeepLinkTarget(stale, "target");
  const coveredCount = buildDayGroupBoundaries(stale).reduce(
    (n, g) => n + g.count,
    0,
  );
  const latestKey = selectLatestMessageKey(stale);

  assert.equal(link.resolved, false);
  assert.equal(coveredCount, stale.length);
  assert.equal(latestKey, "b");
});

// --- deferred reply-list render state (thread side pane) --------------------
//
// When MessageThreadPanel gates its reply render behind useDeferredValue, the
// painted (deferred) snapshot lags the live one for a frame. selectDeferredList
// RenderState picks which of three states the reply region paints so we never
// flash "No replies" over a list that's streaming in on the deferred commit.

test("deferred-render: paints the list whenever the deferred snapshot has rows", () => {
  // deferred caught up — normal steady state.
  assert.equal(selectDeferredListRenderState(3, 3), "list");
  // deferred still showing the OLD non-empty list while a new one streams in;
  // we keep painting rows (no flash) — the dim-pending styling handles the lag.
  assert.equal(selectDeferredListRenderState(2, 5), "list");
});

test("deferred-render: empty state only when the LIVE list is genuinely empty", () => {
  // Both empty — the thread truly has no replies.
  assert.equal(selectDeferredListRenderState(0, 0), "empty");
});

test("deferred-render: pending when deferred is empty but live has content", () => {
  // Deferred snapshot hasn't committed the rows yet but the live list is
  // non-empty. Must NOT report "empty" — that would flash the "No replies"
  // affordance for a frame on thread-open.
  assert.equal(selectDeferredListRenderState(0, 4), "pending");
  assert.notEqual(selectDeferredListRenderState(0, 4), "empty");
});

test("deferred-render: keys the empty decision off the live count, not deferred", () => {
  // Same deferred count (0), opposite verdicts — proving the live count is the
  // tie-breaker. This is the no-tearing guarantee for the empty affordance:
  // the empty state is a function of the LIVE list, never the lagging one.
  assert.equal(selectDeferredListRenderState(0, 0), "empty");
  assert.equal(selectDeferredListRenderState(0, 1), "pending");
});

test("timeline-body-surface: loading and deferred-pending both paint the single static skeleton", () => {
  assert.equal(
    selectTimelineBodySurface({
      deferredCount: 0,
      isLoading: true,
      liveCount: 0,
    }),
    "skeleton",
  );
  assert.equal(
    selectTimelineBodySurface({
      deferredCount: 0,
      isLoading: false,
      liveCount: 3,
    }),
    "skeleton",
  );
});

test("timeline-body-surface: deferred rows paint the message list", () => {
  assert.equal(
    selectTimelineBodySurface({
      deferredCount: 2,
      isLoading: false,
      liveCount: 2,
    }),
    "list",
  );
});

test("timeline-body-surface: empty only when live and deferred rows are empty", () => {
  assert.equal(
    selectTimelineBodySurface({
      deferredCount: 0,
      isLoading: false,
      liveCount: 0,
    }),
    "empty",
  );
});

test("deferred-snapshot: stale when channel ids diverge during channel switch", () => {
  assert.equal(
    isDeferredTimelineSnapshotStale({
      deferredSnapshot: { channelId: "chan-a" },
      liveSnapshot: { channelId: "chan-b" },
    }),
    true,
  );
});

test("deferred-snapshot: fresh when channel ids match", () => {
  assert.equal(
    isDeferredTimelineSnapshotStale({
      deferredSnapshot: { channelId: "chan-a" },
      liveSnapshot: { channelId: "chan-a" },
    }),
    false,
  );
});

test("timeline-body-surface: stale deferred channel snapshot paints skeleton instead of old list", () => {
  const isStale = isDeferredTimelineSnapshotStale({
    deferredSnapshot: { channelId: "chan-a" },
    liveSnapshot: { channelId: "chan-b" },
  });

  assert.equal(
    selectTimelineBodySurface({
      deferredCount: 4,
      isLoading: isStale,
      liveCount: 0,
    }),
    "skeleton",
  );
});

test("timeline-intro-surface: skeleton suppresses intro while loading", () => {
  assert.equal(
    selectTimelineIntroSurface({
      hasChannelIntro: true,
      hasDirectMessageIntro: false,
      hasReachedChannelStart: true,
      isSkeletonVisible: true,
    }),
    null,
  );
});

test("timeline-intro-surface: intro may coexist with the message list", () => {
  assert.equal(
    selectTimelineBodySurface({
      deferredCount: 2,
      isLoading: false,
      liveCount: 2,
    }),
    "list",
  );
  assert.equal(
    selectTimelineIntroSurface({
      hasChannelIntro: true,
      hasDirectMessageIntro: false,
      hasReachedChannelStart: true,
      isSkeletonVisible: false,
    }),
    "channel-intro",
  );
});

test("timeline-intro-surface: channel intro waits for oldest-history boundary", () => {
  assert.equal(
    selectTimelineIntroSurface({
      hasChannelIntro: true,
      hasDirectMessageIntro: false,
      hasReachedChannelStart: false,
      isSkeletonVisible: false,
    }),
    null,
  );
});

test("timeline-intro-surface: direct-message intro wins over channel intro", () => {
  assert.equal(
    selectTimelineIntroSurface({
      hasChannelIntro: true,
      hasDirectMessageIntro: true,
      hasReachedChannelStart: false,
      isSkeletonVisible: false,
    }),
    "direct-message-intro",
  );
});

test("timeline-intro-surface: no intro without an intro model", () => {
  assert.equal(
    selectTimelineIntroSurface({
      hasChannelIntro: false,
      hasDirectMessageIntro: false,
      hasReachedChannelStart: true,
      isSkeletonVisible: false,
    }),
    null,
  );
});

test("isRenderedTimelineBehindHistoryPrepend: false when both empty", () => {
  assert.equal(isRenderedTimelineBehindHistoryPrepend([], []), false);
});

test("isRenderedTimelineBehindHistoryPrepend: false during initial empty-to-loaded settle", () => {
  // Rendered still empty while the live cache filled on open: not a prepend lag,
  // so a freshly opened short channel can still show its intro.
  assert.equal(
    isRenderedTimelineBehindHistoryPrepend([], [{ id: "a" }]),
    false,
  );
});

test("isRenderedTimelineBehindHistoryPrepend: false when rendered matches live", () => {
  const a = { id: "a" };
  const b = { id: "b" };
  assert.equal(isRenderedTimelineBehindHistoryPrepend([a, b], [a, b]), false);
});

test("isRenderedTimelineBehindHistoryPrepend: true when rendered trails a live prepend", () => {
  const older = { id: "older" };
  const a = { id: "a" };
  const b = { id: "b" };
  // Live cache gained an older root; rendered still starts at `a` and is shorter.
  assert.equal(
    isRenderedTimelineBehindHistoryPrepend([a, b], [older, a, b]),
    true,
  );
});

test("isRenderedTimelineBehindHistoryPrepend: false when rendered oldest already matches live oldest", () => {
  const a = { id: "a" };
  const b = { id: "b" };
  // Rendered shorter than live but oldest unchanged (e.g. a newer live append):
  // not behind an older-history prepend.
  assert.equal(isRenderedTimelineBehindHistoryPrepend([a], [a, b]), false);
});
