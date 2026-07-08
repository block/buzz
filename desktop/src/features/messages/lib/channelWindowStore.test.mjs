import assert from "node:assert/strict";
import test from "node:test";
import {
  appendOlderChannelWindow,
  channelWindowThreadSummaries,
  emptyChannelWindowStore,
  flattenChannelWindowEvents,
  mergeLiveChannelWindowEvent,
  mergeLiveThreadSummary,
  replaceNewestChannelWindow,
} from "./channelWindowStore.ts";

function event(id, createdAt, kind = 9) {
  return {
    id: id.padEnd(64, "0"),
    pubkey: "a".repeat(64),
    created_at: createdAt,
    kind,
    tags: [["h", "channel"]],
    content: id,
    sig: "b".repeat(128),
  };
}
const cursor = (item) => ({ createdAt: item.created_at, eventId: item.id });
function page(startCursor, rows, { aux = [], hasMore = true } = {}) {
  return {
    startCursor,
    rows: rows.map((item) => ({ event: item, thread: null })),
    aux,
    nextCursor: hasMore ? cursor(rows.at(-1)) : null,
    hasMore,
  };
}

test("dense-second pages form a lossless cursor chain", () => {
  const first = page(null, [event("a", 100), event("b", 100)]);
  const second = page(first.nextCursor, [event("c", 100), event("z", 99)], {
    hasMore: false,
  });
  const store = appendOlderChannelWindow(
    replaceNewestChannelWindow(emptyChannelWindowStore(), first),
    second,
  );
  assert.deepEqual(
    flattenChannelWindowEvents(store).map((item) => item.content),
    ["z", "c", "b", "a"],
  );
});

test("accepts a relay cursor beyond the last reconstructed row", () => {
  const visible = event("a", 100);
  const skippedRawTail = event("z", 99);
  const first = {
    ...page(null, [visible]),
    nextCursor: cursor(skippedRawTail),
  };
  const store = replaceNewestChannelWindow(emptyChannelWindowStore(), first);
  const complete = appendOlderChannelWindow(
    store,
    page(first.nextCursor, [event("older", 98)], { hasMore: false }),
  );

  assert.deepEqual(store.pages[0].nextCursor, cursor(skippedRawTail));
  assert.deepEqual(
    flattenChannelWindowEvents(complete).map((item) => item.content),
    ["older", "a"],
  );
});

test("accepts a relay cursor when all retained rows were skipped", () => {
  const first = page(null, [event("head", 110)]);
  const initial = replaceNewestChannelWindow(emptyChannelWindowStore(), first);
  const skippedRawTail = event("z", 99);
  const next = appendOlderChannelWindow(initial, {
    startCursor: first.nextCursor,
    rows: [],
    aux: [],
    nextCursor: cursor(skippedRawTail),
    hasMore: true,
  });

  assert.deepEqual(next.pages[1].nextCursor, cursor(skippedRawTail));
});

test("rejects a response that does not continue the echoed cursor", () => {
  const first = page(null, [event("a", 100)]);
  const store = replaceNewestChannelWindow(emptyChannelWindowStore(), first);
  assert.throws(
    () =>
      appendOlderChannelWindow(
        store,
        page(cursor(event("x", 50)), [event("z", 49)], { hasMore: false }),
      ),
    /does not continue/,
  );
});

test("rejects inconsistent exhaustion and cursor facts", () => {
  const row = event("a", 100);
  assert.throws(
    () =>
      replaceNewestChannelWindow(emptyChannelWindowStore(), {
        startCursor: null,
        rows: [{ event: row, thread: null }],
        aux: [],
        nextCursor: cursor(row),
        hasMore: false,
      }),
    /disagree/,
  );
});

test("newest refresh drops a stale tail when its boundary moves", () => {
  const first = page(null, [event("a", 100)]);
  const loaded = appendOlderChannelWindow(
    replaceNewestChannelWindow(emptyChannelWindowStore(), first),
    page(first.nextCursor, [event("z", 90)], { hasMore: false }),
  );
  const refreshed = replaceNewestChannelWindow(
    loaded,
    page(null, [event("n", 110), event("a", 100)]),
  );
  assert.equal(refreshed.pages.length, 1);
  assert.deepEqual(
    flattenChannelWindowEvents(refreshed).map((item) => item.content),
    ["a", "n"],
  );
});

test("live rows arriving before page zero enter the overlay", () => {
  const live = event("n", 110);
  const store = mergeLiveChannelWindowEvent(emptyChannelWindowStore(), live);

  assert.deepEqual(store.liveOverlay, [live]);
  assert.deepEqual(
    flattenChannelWindowEvents(store).map((item) => item.content),
    ["n"],
  );
});

test("live backdated rows stay outside pages and render in order", () => {
  const store = replaceNewestChannelWindow(
    emptyChannelWindowStore(),
    page(null, [event("n", 110), event("a", 100)]),
  );
  const withLive = mergeLiveChannelWindowEvent(store, event("m", 105));
  assert.equal(withLive.pages[0], store.pages[0]);
  assert.deepEqual(
    flattenChannelWindowEvents(withLive).map((item) => item.content),
    ["a", "m", "n"],
  );
});

test("live rows below the oldest retained boundary wait for paging", () => {
  const store = replaceNewestChannelWindow(
    emptyChannelWindowStore(),
    page(null, [event("n", 110), event("a", 100)]),
  );
  assert.equal(mergeLiveChannelWindowEvent(store, event("old", 90)), store);
});

test("live aux stays separate from authoritative page closure", () => {
  const store = replaceNewestChannelWindow(
    emptyChannelWindowStore(),
    page(null, [event("a", 100)]),
  );
  const aux = event("reaction", 110, 7);
  const withAux = mergeLiveChannelWindowEvent(store, aux, false);

  assert.deepEqual(withAux.pages, store.pages);
  assert.deepEqual(withAux.liveAux, [aux]);
  assert.equal(
    flattenChannelWindowEvents(withAux).filter((item) => item.id === aux.id)
      .length,
    1,
  );
  assert.equal(mergeLiveChannelWindowEvent(withAux, aux, false), withAux);
});

test("authoritative refresh reconciles duplicate live rows", () => {
  const initial = replaceNewestChannelWindow(
    emptyChannelWindowStore(),
    page(null, [event("a", 100)]),
  );
  const withLive = mergeLiveChannelWindowEvent(initial, event("n", 110));
  const refreshed = replaceNewestChannelWindow(
    withLive,
    page(null, [event("n", 110), event("a", 100)]),
  );
  assert.deepEqual(refreshed.liveOverlay, []);
  assert.equal(
    flattenChannelWindowEvents(refreshed).filter((item) => item.content === "n")
      .length,
    1,
  );
});

test("older-page append reconciles a live row pushed below page zero", () => {
  const initial = replaceNewestChannelWindow(
    emptyChannelWindowStore(),
    page(null, [event("a", 100)]),
  );
  const live = event("n", 110);
  const withLive = mergeLiveChannelWindowEvent(initial, live);
  const refreshed = replaceNewestChannelWindow(
    withLive,
    page(null, [event("newer", 120)]),
  );
  const reconciled = appendOlderChannelWindow(
    refreshed,
    page(refreshed.pages[0].nextCursor, [live], { hasMore: false }),
  );

  assert.deepEqual(reconciled.liveOverlay, []);
  assert.equal(
    flattenChannelWindowEvents(reconciled).filter((item) => item.id === live.id)
      .length,
    1,
  );
});

// Sharper than the count-only reconcile checks above: when the live-overlay copy
// and the authoritative relay copy share an id but DIFFER in content (an
// optimistic/pending row later re-served by the relay in an older page), the
// rendered row must be the relay copy. `flattenChannelWindowEvents` sets page
// rows before liveOverlay, so an un-reconciled overlay entry shadows the
// authoritative row — user sees the stale/pending version after paginating.
test("older-page append: authoritative relay row wins over a stale overlay copy", () => {
  const withContent = (id, createdAt, content) => ({
    ...event(id, createdAt),
    content,
    pending: content.startsWith("PENDING"),
  });
  const initial = replaceNewestChannelWindow(
    emptyChannelWindowStore(),
    page(null, [event("a", 100)]),
  );
  const staleOverlay = withContent("n", 110, "PENDING n");
  const withLive = mergeLiveChannelWindowEvent(initial, staleOverlay);
  const refreshed = replaceNewestChannelWindow(
    withLive,
    page(null, [event("newer", 120)]),
  );
  const authoritative = withContent("n", 110, "CONFIRMED n");
  const reconciled = appendOlderChannelWindow(
    refreshed,
    page(refreshed.pages[0].nextCursor, [authoritative], { hasMore: false }),
  );

  const rows = flattenChannelWindowEvents(reconciled).filter(
    (item) => item.id === staleOverlay.id,
  );
  assert.equal(rows.length, 1);
  assert.equal(rows[0].content, "CONFIRMED n");
  assert.deepEqual(reconciled.liveOverlay, []);
});

test("flattening dedupes aux closure events returned on adjacent pages", () => {
  const deletion = event("d", 120, 5);
  const first = page(null, [event("a", 100)], { aux: [deletion] });
  const store = appendOlderChannelWindow(
    replaceNewestChannelWindow(emptyChannelWindowStore(), first),
    page(first.nextCursor, [event("z", 90)], {
      aux: [deletion],
      hasMore: false,
    }),
  );
  assert.equal(
    flattenChannelWindowEvents(store).filter((item) => item.id === deletion.id)
      .length,
    1,
  );
});

const summary = (replyCount) => ({
  replyCount,
  descendantCount: replyCount,
  lastReplyAt: 500,
  participantPubkeys: ["c".repeat(64)],
});
const live = (replyCount, createdAt) => ({
  summary: summary(replyCount),
  createdAt,
});

test("live thread summary overlays the page summary for its root", () => {
  const root = event("a", 100);
  const first = page(null, [root], { hasMore: false });
  first.rows[0].thread = summary(1);
  let store = replaceNewestChannelWindow(emptyChannelWindowStore(), first);
  store = mergeLiveThreadSummary(store, root.id, live(2, 200));
  assert.equal(channelWindowThreadSummaries(store).get(root.id).replyCount, 2);
});

test("live thread summary gives a badge to roots with no page summary", () => {
  const store = mergeLiveThreadSummary(
    replaceNewestChannelWindow(
      emptyChannelWindowStore(),
      page(null, [event("a", 100)], { hasMore: false }),
    ),
    event("a", 100).id,
    live(1, 200),
  );
  const summaries = channelWindowThreadSummaries(store);
  assert.equal(summaries.get(event("a", 100).id).replyCount, 1);
});

test("stale live thread summaries never roll a newer one back", () => {
  const rootId = event("a", 100).id;
  const store = mergeLiveThreadSummary(
    emptyChannelWindowStore(),
    rootId,
    live(3, 300),
  );
  // Same-timestamp and older pushes are both ignored; identity proves no churn.
  assert.equal(mergeLiveThreadSummary(store, rootId, live(2, 300)), store);
  assert.equal(mergeLiveThreadSummary(store, rootId, live(2, 250)), store);
  assert.equal(channelWindowThreadSummaries(store).get(rootId).replyCount, 3);
});

test("head refetch clears live summaries in favor of the fresh snapshot", () => {
  const root = event("a", 100);
  let store = mergeLiveThreadSummary(
    emptyChannelWindowStore(),
    root.id,
    live(7, 200),
  );
  const refreshed = page(null, [root], { hasMore: false });
  refreshed.rows[0].thread = summary(4);
  store = replaceNewestChannelWindow(store, refreshed);
  assert.equal(channelWindowThreadSummaries(store).get(root.id).replyCount, 4);
});

test("live summaries survive scrollback pages and still win for their root", () => {
  const head = event("b", 200);
  const older = event("a", 100);
  const first = page(null, [head]);
  let store = replaceNewestChannelWindow(emptyChannelWindowStore(), first);
  store = mergeLiveThreadSummary(store, older.id, live(5, 300));
  const tail = page(first.nextCursor, [older], { hasMore: false });
  tail.rows[0].thread = summary(4);
  store = appendOlderChannelWindow(store, tail);
  assert.equal(channelWindowThreadSummaries(store).get(older.id).replyCount, 5);
});
