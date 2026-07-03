import assert from "node:assert/strict";
import test from "node:test";
import {
  appendOlderChannelWindow,
  emptyChannelWindowStore,
  flattenChannelWindowEvents,
  mergeLiveChannelWindowEvent,
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
